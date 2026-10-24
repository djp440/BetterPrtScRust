use bps_platform::{Display, FrameSource, Image};

use crate::{CaptureMode, CoreError};

/// 一次捕获的产物（截图）及其来源信息。
pub(crate) struct Screenshot {
    pub image: Image,
}

/// 这个捕获模式是否已经实现。实现新的捕获模式时在这里登记。
pub(crate) fn is_implemented(mode: CaptureMode) -> bool {
    matches!(mode, CaptureMode::Display)
}

/// 按捕获模式从帧源取得截图。
pub(crate) fn capture(
    frames: &dyn FrameSource,
    mode: CaptureMode,
) -> Result<Screenshot, CoreError> {
    match mode {
        CaptureMode::Display => {
            let display = display_under_cursor(frames)?;
            let image = frames.capture_display(display.id)?;
            Ok(Screenshot { image })
        }
        unsupported => Err(CoreError::UnsupportedCaptureMode(unsupported)),
    }
}

/// 鼠标所在的显示器；鼠标不在任何显示器内时退回主显示器。
fn display_under_cursor(frames: &dyn FrameSource) -> Result<Display, CoreError> {
    let displays = frames.displays()?;
    let cursor = frames.cursor_position()?;
    displays
        .iter()
        .find(|d| d.bounds.contains(cursor))
        .or_else(|| displays.iter().find(|d| d.is_primary))
        .or_else(|| displays.first())
        .cloned()
        .ok_or(CoreError::NoDisplay)
}
