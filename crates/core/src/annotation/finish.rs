//! 标注的结束方式：复制到剪贴板、自动存储、另存为。
//!
//! 复制和自动存储直接复用截后动作（`action::run`），遵守同样的剪贴板和命名规则；绑定里
//! 除标注外还有复制或自动存储时，无论选哪种结束方式它们也一并执行（例如绑定了“标注 +
//! 自动存储”，点“复制”时既复制也自动存储，剪贴板因此附带存下的文件）。

use std::path::Path;

use bps_platform::{FileType, Notification, Platform, SaveAsOptions};

use super::session::Finish;
use crate::capture::{CaptureFailure, Screenshot};
use crate::filename::FilenameFields;
use crate::{CaptureRequest, CoreError, OutputFormat, PostCaptureAction, Texts, action, encode};

/// 另存为对话框里可选的格式（显示名、扩展名），按这个顺序列出。
const SAVE_AS_FORMATS: [(OutputFormat, &str, &[&str]); 4] = [
    (OutputFormat::Png, "PNG", &["png"]),
    (OutputFormat::Jpeg, "JPEG", &["jpg", "jpeg"]),
    (OutputFormat::WebP, "WebP", &["webp"]),
    (OutputFormat::Avif, "AVIF", &["avif"]),
];

/// 另存为对话框的初始内容：文件名按命名模板生成，类型默认是输出格式，
/// 目录默认是自动存储目录。
pub(crate) fn save_as_options(
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
        file_types: SAVE_AS_FORMATS
            .iter()
            .map(|(_, name, extensions)| FileType {
                name: texts.format("save-as-file-type", &[("format", name)]),
                extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
            })
            .collect(),
        default_file_type: SAVE_AS_FORMATS
            .iter()
            .position(|(format, ..)| *format == output.output_format)
            .unwrap_or(0),
    }
}

/// 按结束方式输出标注好的截图。另存为时 `save_as` 是用户在对话框里选的路径。
pub(crate) fn run(
    platform: &mut Platform,
    screenshot: &Screenshot,
    request: &CaptureRequest,
    finish: Finish,
    save_as: Option<&Path>,
    texts: &Texts,
) -> Result<(), CaptureFailure> {
    let mut actions: Vec<PostCaptureAction> = request
        .actions
        .iter()
        .copied()
        .filter(|a| {
            matches!(
                a,
                PostCaptureAction::CopyToClipboard | PostCaptureAction::AutoSave
            )
        })
        .collect();
    let chosen = match finish {
        Finish::CopyToClipboard => Some(PostCaptureAction::CopyToClipboard),
        Finish::AutoSave => Some(PostCaptureAction::AutoSave),
        Finish::SaveAs => None,
    };
    if let Some(action) = chosen
        && !actions.contains(&action)
    {
        actions.push(action);
    }
    let effective = CaptureRequest {
        actions,
        ..request.clone()
    };

    let saved_as = save_as.map(|path| {
        let result = write_save_as(platform, screenshot, request.output.output_format, path);
        if let Err(error) = &result {
            platform.notifier.notify(Notification {
                title: texts.get("notification-save-as-failed-title"),
                body: error.localized(texts),
                on_click: None,
            });
        }
        result
    });
    let ran = action::run(platform, screenshot, &effective, texts);
    match saved_as {
        Some(Err(error)) => Err(CaptureFailure {
            error,
            notified: true,
        }),
        _ => ran,
    }
}

/// 按路径的扩展名（不认识时用输出格式）编码，写到用户选的路径，覆盖已有文件。
fn write_save_as(
    platform: &mut Platform,
    screenshot: &Screenshot,
    default: OutputFormat,
    path: &Path,
) -> Result<(), CoreError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = SAVE_AS_FORMATS
        .iter()
        .find(|(_, _, extensions)| {
            extension
                .as_deref()
                .is_some_and(|e| extensions.contains(&e))
        })
        .map_or(default, |(format, ..)| *format);
    let encoded = encode::encode(&screenshot.image, format)?;
    platform.file_system.write(path, &encoded)?;
    Ok(())
}
