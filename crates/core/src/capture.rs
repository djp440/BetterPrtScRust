use bps_platform::{Display, FrameSource, Image, LocalDateTime, Platform};

use crate::{CaptureMode, CaptureRequest, CoreError, PostCaptureAction, stitch};

/// 一次捕获的产物（截图）及其来源信息。
pub(crate) struct Screenshot {
    pub image: Image,
    pub mode: CaptureMode,
    /// 捕获发生时的本地时间。
    pub captured_at: LocalDateTime,
    /// 被捕获窗口的标题；不是窗口模式时为 `None`。
    pub window_title: Option<String>,
}

/// 一次捕获失败的原因，以及是否已经就这次失败通知过用户。
pub(crate) struct CaptureFailure {
    pub error: CoreError,
    pub notified: bool,
}

impl CaptureFailure {
    /// 截后动作之前（捕获本身）的失败：还没有通知过用户。
    pub fn before_actions(error: impl Into<CoreError>) -> Self {
        Self {
            error: error.into(),
            notified: false,
        }
    }

    /// 截后动作失败：做了自动存储时，截后动作已经就结果发过通知（“截图已保存”或“自动存储失败”）。
    pub fn in_actions(error: CoreError, request: &CaptureRequest) -> Self {
        Self {
            error,
            notified: request.actions.contains(&PostCaptureAction::AutoSave),
        }
    }
}

/// 这个捕获模式是否已经实现。实现新的捕获模式时在这里登记。
pub(crate) fn is_implemented(mode: CaptureMode) -> bool {
    matches!(
        mode,
        CaptureMode::Display | CaptureMode::VirtualDesktop | CaptureMode::Region
    )
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
        // 区域需要用户在选区遮罩上框选，由 `region::RegionCapture` 跨多个事件完成，不走这里。
        CaptureMode::Region | CaptureMode::Window => Err(CoreError::UnsupportedCaptureMode(mode)),
    }
}

/// 鼠标所在的显示器；鼠标不在任何显示器内时退回主显示器。
pub(crate) fn display_under_cursor(frames: &dyn FrameSource) -> Result<Display, CoreError> {
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
