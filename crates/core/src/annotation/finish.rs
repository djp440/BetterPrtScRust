//! 等待确认界面的结束方式：确认（复制到剪贴板）、另存为；以及把截图交给 OCR / 翻译。
//!
//! 确认直接复用交付逻辑（`action::run`），遵守同样的剪贴板和命名规则：绑定开了自动保存时
//! 同时存盘，剪贴板因此附带存下的文件。另存为只写用户选的那个文件；OCR、翻译只打开结果面板，
//! 都不复制、不自动保存。

use std::path::{Path, PathBuf};

use bps_platform::{FileType, LocalDateTime, Notification, Platform, Rect, SaveAsOptions};

use super::session::{Annotator, Finish};
use crate::action::AfterRecognition;
use crate::capture::{CaptureFailure, Screenshot};
use crate::filename::FilenameFields;
use crate::{CaptureRequest, OutputFormat, Texts, action};

/// 被标注的截图是什么时候、从哪里截下来的：命名模板用它。
pub(crate) struct Origin {
    pub captured_at: LocalDateTime,
    /// 窗口捕获时被截窗口的标题。
    pub window_title: Option<String>,
}

/// 另存为对话框的初始内容：文件名按命名模板生成，类型默认是输出格式，
/// 目录默认是自动存储目录。
fn save_as_options(
    platform: &Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    texts: &Texts,
) -> SaveAsOptions {
    let output = &request.output;
    let stem = output.filename_template.render(&FilenameFields {
        captured_at: screenshot.captured_at,
        mode: screenshot.mode,
        window_title: screenshot.window_title.as_deref(),
    });
    SaveAsOptions {
        file_name: format!("{stem}.{}", output.output_format.extension()),
        folder: action::save_dir(platform, output).ok(),
        file_types: OutputFormat::ALL
            .iter()
            .map(|format| FileType {
                name: texts.format("save-as-file-type", &[("format", &format.display_name())]),
                extensions: format
                    .extensions()
                    .iter()
                    .map(|e| (*e).to_owned())
                    .collect(),
            })
            .collect(),
        default_file_type: OutputFormat::ALL
            .iter()
            .position(|format| *format == output.output_format)
            .unwrap_or(0),
    }
}

/// 用户在标注工具条上选择了结束方式：取得合成了标注的截图（`region` 是标注所在的区域），
/// 另存为时先弹出对话框（遮罩还在），然后关闭遮罩，按结束方式输出。
///
/// 用户在对话框里取消时返回 `None`：遮罩不关，回到标注。
pub(crate) fn conclude(
    platform: &mut Platform,
    annotator: &Annotator,
    region: Rect,
    request: &CaptureRequest,
    origin: &Origin,
    finish: Finish,
    texts: &'static Texts,
) -> Option<Result<(), CaptureFailure>> {
    let image = match annotator.final_image(platform, region) {
        Ok(image) => image,
        Err(e) => {
            platform.overlay.close();
            return Some(Err(CaptureFailure::before_actions(e)));
        }
    };
    let screenshot = Screenshot {
        image,
        mode: request.mode,
        captured_at: origin.captured_at,
        window_title: origin.window_title.clone(),
    };
    let mut save_as: Option<PathBuf> = None;
    if finish == Finish::SaveAs {
        let options = save_as_options(platform, &screenshot, request, texts);
        match platform.file_dialog.save_as(&options) {
            Ok(Some(path)) => save_as = Some(path),
            Ok(None) => return None,
            Err(e) => {
                platform.overlay.close();
                return Some(Err(CaptureFailure::before_actions(e)));
            }
        }
    }
    platform.overlay.close();
    Some(run(
        platform,
        &screenshot,
        request,
        finish,
        save_as.as_deref(),
        texts,
    ))
}

/// 按结束方式输出标注好的截图。另存为时 `save_as` 是用户在对话框里选的路径。
fn run(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    finish: Finish,
    save_as: Option<&Path>,
    texts: &'static Texts,
) -> Result<(), CaptureFailure> {
    match (finish, save_as) {
        (Finish::SaveAs, Some(path)) => write_save_as(
            platform,
            screenshot,
            request.output.output_format,
            path,
            texts,
        ),
        _ => action::run(platform, screenshot, request, texts),
    }
}

/// 用户在工具条上点了 OCR（`translate` 为假）或翻译：取得合成了标注的截图（`region` 是标注所在的
/// 区域），关闭遮罩，交给结果面板。OCR 在开了“识别后自动复制”（`auto_copy`）时只复制文字、
/// 不显示面板；翻译在同一选项开启时只复制译文。
pub(crate) fn recognize(
    platform: &mut Platform,
    annotator: &Annotator,
    region: Rect,
    translate: bool,
    auto_copy: bool,
    texts: &'static Texts,
) -> Result<(), CaptureFailure> {
    let image = annotator.final_image(platform, region);
    platform.overlay.close();
    let image = image.map_err(CaptureFailure::before_actions)?;
    let then = match (translate, auto_copy) {
        (true, true) => AfterRecognition::TranslateCopy,
        (true, false) => AfterRecognition::Translate,
        (false, true) => AfterRecognition::Copy,
        (false, false) => AfterRecognition::Show,
    };
    action::open_result_panel(platform, image, then, texts)
        .map_err(CaptureFailure::already_notified)
}

/// 按路径的扩展名（不认识时用输出格式）编码，写到用户选的路径，覆盖已有文件。
/// 慢格式在后台编码，写文件和失败通知在编码完成后进行。
fn write_save_as(
    platform: &mut Platform,
    screenshot: &Screenshot,
    default: OutputFormat,
    path: &Path,
    texts: &'static Texts,
) -> Result<(), CaptureFailure> {
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(OutputFormat::from_extension)
        .unwrap_or(default);
    let path = path.to_owned();
    action::encode_then(platform, screenshot, format, move |platform, _, encoded| {
        let written = encoded.and_then(|bytes| Ok(platform.file_system.write(&path, &bytes)?));
        written.map_err(|error| {
            platform.notifier.notify(Notification {
                title: texts.get("notification-save-as-failed-title"),
                body: error.localized(texts),
                on_click: None,
            });
            CaptureFailure::already_notified(error)
        })
    })
}
