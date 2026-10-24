//! 窗口捕获：取窗口布局并拍冻结帧 → 显示选区遮罩 → 用户悬停高亮、单击选中顶层窗口 →
//! 关闭遮罩后截取这个窗口的完整内容 → 截后动作。
//!
//! 与区域捕获一样跨越多个平台事件：[`WindowCapture::start`] 打开遮罩后立即返回，之后每个
//! [`OverlayInput`] 交给 [`WindowCapture::handle`]，直到用户选中窗口或取消。
//! 窗口内容来自 `WindowSource::capture_window`（被遮挡的部分也在内），不从冻结帧裁剪；
//! 冻结帧只是选择时的背景。

use bps_platform::{
    Backdrop, IncomingRequest, LocalDateTime, OverlayInput, OverlayScene, Platform, PlatformError,
    TopLevelWindow,
};

use crate::capture::{CaptureFailure, Screenshot};
use crate::region::Ending;
use crate::window_selection::{WindowSelection, WindowStep};
use crate::{CaptureMode, CaptureRequest, CoreError, Texts, action};

/// 进行中的窗口捕获。
pub(crate) struct WindowCapture {
    request: CaptureRequest,
    /// 拍下冻结帧的时间，即这次捕获发生的时间。
    captured_at: LocalDateTime,
    selection: WindowSelection,
    /// 遮罩上当前画着的场景；场景没变时不重画。
    drawn: Option<OverlayScene>,
    /// 开始捕获时的界面语言：截后动作的通知用它。
    texts: &'static Texts,
    /// 经 CLI 转交来、等捕获结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

impl WindowCapture {
    /// 取得此刻的窗口布局、拍下冻结帧并显示选区遮罩；截后动作的通知用 `texts` 的语言。
    pub fn start(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
    ) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        let layout = platform.window_source.layout()?;
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        // 遮罩一出现就高亮光标下的窗口；读不到光标位置时等第一次移动。
        let cursor = platform.frame_source.cursor_position().ok();
        let Some(selection) = WindowSelection::new(displays, layout, cursor) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        let mut capture = Self {
            request,
            captured_at,
            selection,
            drawn: None,
            texts,
            requester: None,
        };
        if let Err(e) = capture.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(capture)
    }

    /// 处理用户在遮罩上的一次输入。返回 `None` 表示还在选择；否则这次捕获已经结束，遮罩已关闭。
    pub fn handle(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self.selection.handle(input) {
            WindowStep::Continue => match self.redraw(platform) {
                Ok(()) => None,
                Err(e) => {
                    platform.overlay.close();
                    Some(Err(CaptureFailure::before_actions(e)))
                }
            },
            WindowStep::Confirmed(window) => {
                Some(self.finish(platform, &window).map(|()| Ending::Captured))
            }
            WindowStep::Cancelled => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
        }
    }

    /// 关闭遮罩，截取 `window` 的完整内容，再执行截后动作。
    fn finish(
        &mut self,
        platform: &mut Platform,
        window: &TopLevelWindow,
    ) -> Result<(), CaptureFailure> {
        platform.overlay.close();
        let image = platform
            .window_source
            .capture_window(window.id)
            .map_err(CaptureFailure::before_actions)?;
        let screenshot = Screenshot {
            image,
            mode: CaptureMode::Window,
            captured_at: self.captured_at,
            window_title: Some(window.title.clone()),
        };
        action::run(platform, &screenshot, &self.request, self.texts)
            .map_err(|error| CaptureFailure::in_actions(error, &self.request))
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        let scene = self.selection.scene();
        if self.drawn.as_ref() != Some(&scene) {
            platform.overlay.render(&scene)?;
            self.drawn = Some(scene);
        }
        Ok(())
    }
}
