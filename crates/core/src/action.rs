//! 截后动作：复制到剪贴板、自动存储。

use std::path::PathBuf;

use bps_platform::{ClipboardItem, NewFile, Notification, NotificationClick, Platform};

use crate::capture::Screenshot;
use crate::filename::FilenameFields;
use crate::output::DEFAULT_SAVE_SUBDIR;
use crate::{
    CaptureRequest, CoreError, OutputFormat, OutputOptions, PostCaptureAction, SaveDir, encode,
};

/// 同名文件最多尝试到 `-N` 的 N。
const MAX_NAME_SUFFIX: u32 = 9999;

/// 对截图执行截后动作。
///
/// 复制到剪贴板和自动存储共用同一份按输出格式编码的结果（AVIF 编码很慢，只编一次）。
/// 先存盘，再一次性写剪贴板，这样剪贴板能附带存下的文件；存储失败不影响复制。
/// 自动存储的结果（成功或失败）都会发通知。
pub(crate) fn run(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
) -> Result<(), CoreError> {
    let copy = request
        .actions
        .contains(&PostCaptureAction::CopyToClipboard);
    let save = request.actions.contains(&PostCaptureAction::AutoSave);
    if !copy && !save {
        return Ok(());
    }

    let format = request.output.output_format;
    let encoded = encode::encode(&screenshot.image, format)?;
    let saved = save.then(|| auto_save(platform, screenshot, &request.output, &encoded));
    let copied = if copy {
        let saved_file = saved.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        copy_to_clipboard(platform, screenshot, format, encoded, saved_file)
    } else {
        Ok(())
    };
    if let Some(saved) = &saved {
        notify_auto_save(platform, saved);
    }
    saved.transpose()?;
    copied
}

fn notify_auto_save(platform: &Platform, saved: &Result<PathBuf, CoreError>) {
    let notification = match saved {
        Ok(path) => Notification {
            title: "截图已保存".to_owned(),
            body: path.display().to_string(),
            on_click: Some(NotificationClick::OpenFile(path.clone())),
        },
        Err(error) => Notification {
            title: "自动存储失败".to_owned(),
            body: error.to_string(),
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
    let image = match format {
        OutputFormat::Png => ClipboardItem::Png(encoded),
        OutputFormat::Jpeg => ClipboardItem::Jpeg(encoded),
        OutputFormat::WebP => ClipboardItem::WebP(encoded),
        OutputFormat::Avif => ClipboardItem::Avif(encoded),
    };
    let mut items = vec![image, ClipboardItem::Dib(encode::dib(&screenshot.image))];
    if let Some(path) = saved {
        items.push(ClipboardItem::Files(vec![path]));
    }
    platform.clipboard.write(items)?;
    Ok(())
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
    let dir = match &output.save_dir {
        SaveDir::Default => platform
            .file_system
            .pictures_dir()?
            .join(DEFAULT_SAVE_SUBDIR),
        SaveDir::Custom(dir) => dir.clone(),
    };
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
