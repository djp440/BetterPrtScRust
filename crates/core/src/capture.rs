use bps_platform::{Display, FrameSource, Image, LocalDateTime, Platform};

use crate::{CaptureMode, CoreError, stitch};

/// 一次捕获的产物（截图）及其来源信息。
pub(crate) struct Screenshot {
    pub image: Image,
    pub mode: CaptureMode,
    /// 捕获发生时的本地时间。
    pub captured_at: LocalDateTime,
    /// 被捕获窗口的标题；不是窗口模式时为 `None`。
    pub window_title: Option<String>,
}

/// 这个捕获模式是否已经实现。实现新的捕获模式时在这里登记。
pub(crate) fn is_implemented(mode: CaptureMode) -> bool {
    matches!(mode, CaptureMode::Display | CaptureMode::VirtualDesktop)
}

/// 按捕获模式从帧源取得截图。
pub(crate) fn capture(platform: &Platform, mode: CaptureMode) -> Result<Screenshot, CoreError> {
    let captured_at = platform.clock.now();
    let frames = platform.frame_source.as_ref();
    match mode {
        CaptureMode::Display => {
            let display = display_under_cursor(frames)?;
            let image = frames.capture_display(display.id)?;
            Ok(Screenshot {
                image,
                mode,
                captured_at,
                window_title: None,
            })
        }
        CaptureMode::VirtualDesktop => {
            let displays = frames.displays()?;
            let bounds = stitch::virtual_desktop_bounds(&displays).ok_or(CoreError::NoDisplay)?;
            let image = stitch::capture_displays(frames, &displays, bounds)?;
            Ok(Screenshot {
                image,
                mode,
                captured_at,
                window_title: None,
            })
        }
        CaptureMode::Region | CaptureMode::Window => Err(CoreError::UnsupportedCaptureMode(mode)),
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
