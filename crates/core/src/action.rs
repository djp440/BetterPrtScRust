//! 截后动作：复制到剪贴板、自动存储、OCR。标注的结束方式也经由这里输出（见 `annotation::finish`）。

use std::path::PathBuf;

use bps_platform::{
    ClipboardItem, Image, NewFile, Notification, NotificationClick, Platform, StandaloneWindow,
};

use crate::capture::{CaptureFailure, Screenshot};
use crate::filename::FilenameFields;
use crate::output::DEFAULT_SAVE_SUBDIR;
use crate::{
    CaptureRequest, CoreError, OutputFormat, OutputOptions, PostCaptureAction, SaveDir, Texts,
    encode,
};

/// 同名文件最多尝试到 `-N` 的 N。
const MAX_NAME_SUFFIX: u32 = 9999;

/// 对截图执行截后动作：先复制到剪贴板、自动存储，再打开结果面板做 OCR
/// （标注、长截屏在此之前已由区域捕获、长截屏会话处理，这里忽略它们）。
///
/// 某个动作失败不影响其他动作，返回第一个失败。通知文字用 `texts` 的语言。
pub(crate) fn run(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    texts: &'static Texts,
) -> Result<(), CaptureFailure> {
    let mut failure = None;
    if let Err(error) = copy_and_save(platform, screenshot, request, texts) {
        failure = Some(error);
    }
    // 翻译动作同样打开结果面板：识别出文字后由用户点“翻译”（只在用户主动点击时才联网）。
    if (request.actions.contains(&PostCaptureAction::Ocr)
        || request.actions.contains(&PostCaptureAction::Translate))
        && let Err(error) = open_result_panel(platform, screenshot.image.clone(), texts)
    {
        failure.get_or_insert(CaptureFailure::already_notified(error));
    }
    failure.map_or(Ok(()), Err)
}

/// OCR：把截图交给一个新的结果面板，由它在本机识别文字并显示；打不开时通知用户。
///
/// 识别在结果面板的子进程里进行，常驻实例不加载 OCR（常驻内存的硬约束）。
fn open_result_panel(
    platform: &mut Platform,
    screenshot: Image,
    texts: &Texts,
) -> Result<(), CoreError> {
    let opened = platform
        .windows
        .open(StandaloneWindow::ResultPanel { screenshot });
    if let Err(e) = &opened {
        platform.notifier.notify(Notification {
            title: texts.get("notification-result-panel-unavailable-title"),
            body: e.to_string(),
            on_click: None,
        });
    }
    Ok(opened?)
}

/// 后台编码完成后在事件循环线程上继续的后续步骤（写剪贴板、存盘、通知等）。
pub(crate) type Continuation = Box<dyn FnOnce(&mut Platform) -> Result<(), CaptureFailure> + Send>;

/// 把 `screenshot` 按 `format` 编码，再交给 `then` 继续。
///
/// 慢格式（AVIF，4K 约 0.9 秒）在后台线程编码，编码期间事件循环照常响应（包括开始下一次
/// 捕获）；完成时平台递交 `BackgroundJobDone`，后续步骤由 [`run_continuations`] 在事件循环线程上
/// 执行，此时本函数早已返回 `Ok`。快格式就地编码、立即继续。
pub(crate) fn encode_then(
    platform: &mut Platform,
    screenshot: &Screenshot,
    format: OutputFormat,
    then: impl FnOnce(
        &mut Platform,
        &Screenshot,
        Result<Vec<u8>, CoreError>,
    ) -> Result<(), CaptureFailure>
    + Send
    + 'static,
) -> Result<(), CaptureFailure> {
    if !format.encodes_slowly() {
        let encoded = encode::encode(&screenshot.image, format);
        return then(platform, screenshot, encoded);
    }
    let screenshot = screenshot.clone();
    platform.background.spawn(Box::new(move || {
        let encoded = encode::encode(&screenshot.image, format);
        let continuation: Continuation =
            Box::new(move |platform| then(platform, &screenshot, encoded));
        Box::new(continuation)
    }));
    Ok(())
}

/// 执行后台任务完成后的后续步骤，返回其中失败的。
pub(crate) fn run_continuations(platform: &mut Platform) -> Vec<CaptureFailure> {
    let mut failures = Vec::new();
    for result in platform.background.take_finished() {
        if let Ok(continuation) = result.downcast::<Continuation>()
            && let Err(failure) = continuation(platform)
        {
            failures.push(failure);
        }
    }
    failures
}

/// 复制到剪贴板与自动存储。
///
/// 两者共用同一份按输出格式编码的结果（AVIF 编码很慢，只编一次，且在后台线程进行）。
/// 先存盘，再一次性写剪贴板，这样剪贴板能附带存下的文件；存储失败不影响复制。
/// 自动存储的结果（成功或失败）都会发通知。
fn copy_and_save(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    texts: &'static Texts,
) -> Result<(), CaptureFailure> {
    let copy = request
        .actions
        .contains(&PostCaptureAction::CopyToClipboard);
    let save = request.actions.contains(&PostCaptureAction::AutoSave);
    if !copy && !save {
        return Ok(());
    }
    let request = request.clone();
    let format = request.output.output_format;
    encode_then(
        platform,
        screenshot,
        format,
        move |platform, screenshot, encoded| store(platform, screenshot, &request, texts, encoded),
    )
}

/// 拿到编码结果之后：自动存储、写剪贴板、通知。失败时 `notified` 说明自动存储是否已通知过。
fn store(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    texts: &Texts,
    encoded: Result<Vec<u8>, CoreError>,
) -> Result<(), CaptureFailure> {
    let copy = request
        .actions
        .contains(&PostCaptureAction::CopyToClipboard);
    let save = request.actions.contains(&PostCaptureAction::AutoSave);
    let failed = |error: CoreError| CaptureFailure {
        error,
        // 做了自动存储时，自动存储已经就结果发过通知（“截图已保存”或“自动存储失败”）。
        notified: save,
    };
    let encoded = match encoded {
        Ok(encoded) => encoded,
        Err(error) => {
            if save {
                notify_auto_save(platform, &Err(error.clone()), texts);
            }
            return Err(failed(error));
        }
    };

    let format = request.output.output_format;
    let saved = save.then(|| auto_save(platform, screenshot, &request.output, &encoded));
    let copied = if copy {
        let saved_file = saved.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        copy_to_clipboard(platform, screenshot, format, encoded, saved_file)
    } else {
        Ok(())
    };
    if let Some(saved) = &saved {
        notify_auto_save(platform, saved, texts);
    }
    if let Some(Err(error)) = saved {
        return Err(failed(error));
    }
    copied.map_err(failed)
}

fn notify_auto_save(platform: &Platform, saved: &Result<PathBuf, CoreError>, texts: &Texts) {
    let notification = match saved {
        Ok(path) => Notification {
            title: texts.get("notification-saved-title"),
            body: path.display().to_string(),
            on_click: Some(NotificationClick::OpenFile(path.clone())),
        },
        Err(error) => Notification {
            title: texts.get("notification-save-failed-title"),
            body: error.localized(texts),
            on_click: None,
        },
    };
    platform.notifier.notify(notification);
}

/// 剪贴板放按输出格式编码的图像，并总是附带 DIB，保证粘贴到任何程序都能用；
/// 这次也做了自动存储时，再附带存下的文件。
fn copy_to_clipboard(
    platform: &mut Platform,
    screenshot: &Screenshot,
    format: OutputFormat,
    encoded: Vec<u8>,
    saved: Option<PathBuf>,
) -> Result<(), CoreError> {
    let image = format.clipboard_item(encoded);
    let mut items = vec![image, ClipboardItem::Dib(encode::dib(&screenshot.image))];
    if let Some(path) = saved {
        items.push(ClipboardItem::Files(vec![path]));
    }
    platform.clipboard.write(items)?;
    Ok(())
}

/// 自动存储的目录（不一定已经存在）。
pub(crate) fn save_dir(platform: &Platform, output: &OutputOptions) -> Result<PathBuf, CoreError> {
    Ok(match &output.save_dir {
        SaveDir::Default => platform
            .file_system
            .pictures_dir()?
            .join(DEFAULT_SAVE_SUBDIR),
        SaveDir::Custom(dir) => dir.clone(),
    })
}

/// 把编码好的截图按命名模板写进存储目录，返回文件路径。
///
/// 同名文件已存在时依次尝试 `名称-1`、`名称-2`……，从不覆盖已有文件。
fn auto_save(
    platform: &mut Platform,
    screenshot: &Screenshot,
    output: &OutputOptions,
    encoded: &[u8],
) -> Result<PathBuf, CoreError> {
    let dir = save_dir(platform, output)?;
    platform.file_system.create_dir_all(&dir)?;

    let stem = output.filename_template.render(&FilenameFields {
        captured_at: screenshot.captured_at,
        mode: screenshot.mode,
        window_title: screenshot.window_title.as_deref(),
    });
    let extension = output.output_format.extension();
    for suffix in 0..=MAX_NAME_SUFFIX {
        let name = match suffix {
            0 => format!("{stem}.{extension}"),
            n => format!("{stem}-{n}.{extension}"),
        };
        let path = dir.join(name);
        if platform.file_system.create_new(&path, encoded)? == NewFile::Created {
            return Ok(path);
        }
    }
    Err(CoreError::TooManySameNameFiles(
        dir.join(format!("{stem}.{extension}")),
    ))
}
