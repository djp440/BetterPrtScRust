use bps_platform::{ClipboardItem, Platform};

use crate::capture::Screenshot;
use crate::{CoreError, PostCaptureAction, encode};

/// 按顺序对截图执行截后动作。
pub(crate) fn run(
    platform: &mut Platform,
    screenshot: &Screenshot,
    actions: &[PostCaptureAction],
) -> Result<(), CoreError> {
    for action in actions {
        match action {
            PostCaptureAction::CopyToClipboard => copy_to_clipboard(platform, screenshot)?,
        }
    }
    Ok(())
}

/// 剪贴板同时放 PNG 和 DIB，保证粘贴到任何程序都能用。
fn copy_to_clipboard(platform: &mut Platform, screenshot: &Screenshot) -> Result<(), CoreError> {
    let items = vec![
        ClipboardItem::Png(encode::png(&screenshot.image)?),
        ClipboardItem::Dib(encode::dib(&screenshot.image)),
    ];
    platform.clipboard.write(items)?;
    Ok(())
}
