//! 区域捕获：拍冻结帧 → 显示选区遮罩 → 用户框选 → 从冻结帧裁剪拼接 → 截后动作。
//!
//! 一次区域捕获跨越多个平台事件（用户的每次输入都是一个事件），所以它是常驻实例持有的
//! 一个进行中的会话，而不是一次函数调用：[`RegionCapture::start`] 打开遮罩后立即返回，
//! 之后每个 [`OverlayInput`] 交给 [`RegionCapture::handle`]，直到用户确定或取消。

use bps_platform::{
    Backdrop, Display, IncomingRequest, LocalDateTime, OverlayInput, OverlayScene, Platform,
    PlatformError,
};

use crate::capture::Screenshot;
use crate::selection::{RegionSelection, Step};
use crate::stitch::{DesktopImage, stitch};
use crate::{CaptureMode, CaptureRequest, CoreError, action};

/// 一次区域捕获是怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// 截图已交给截后动作处理。
    Captured,
    /// 用户取消了，没有任何产物。
    Cancelled,
}

/// 进行中的区域捕获。
pub(crate) struct RegionCapture {
    request: CaptureRequest,
    /// 拍下冻结帧的时间，即这次捕获发生的时间。
    captured_at: LocalDateTime,
    /// 冻结帧覆盖的显示器。
    displays: Vec<Display>,
    selection: RegionSelection,
    /// 遮罩上当前画着的场景；场景没变时不重画。
    drawn: Option<OverlayScene>,
    /// 经 CLI 转交来、等捕获结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

impl RegionCapture {
    /// 拍下冻结帧并显示选区遮罩。
    pub fn start(platform: &mut Platform, request: CaptureRequest) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        let Some(selection) = RegionSelection::new(displays.clone()) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        let mut capture = Self {
            request,
            captured_at,
            displays,
            selection,
            drawn: None,
            requester: None,
        };
        if let Err(e) = capture.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(capture)
    }

    /// 处理用户在遮罩上的一次输入。返回 `None` 表示还在框选；否则这次捕获已经结束，遮罩已关闭。
    pub fn handle(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CoreError>> {
        match self.selection.handle(input) {
            Step::Continue => match self.redraw(platform) {
                Ok(()) => None,
                Err(e) => {
                    platform.overlay.close();
                    Some(Err(e.into()))
                }
            },
            Step::Confirmed(region) => {
                Some(self.finish(platform, region).map(|()| Ending::Captured))
            }
            Step::Cancelled => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
        }
    }

    /// 从冻结帧裁剪拼接出 `region` 的截图，关闭遮罩，再执行截后动作。
    fn finish(
        &mut self,
        platform: &mut Platform,
        region: bps_platform::Rect,
    ) -> Result<(), CoreError> {
        let parts: Result<Vec<DesktopImage>, PlatformError> = self
            .displays
            .iter()
            .filter(|display| display.bounds.intersection(region).is_some())
            .map(|display| {
                Ok(DesktopImage {
                    origin: bps_platform::Point::new(display.bounds.x, display.bounds.y),
                    image: platform.overlay.frozen_image(display.id)?,
                })
            })
            .collect();
        platform.overlay.close();
        let screenshot = Screenshot {
            image: stitch(region, &parts?),
            mode: CaptureMode::Region,
            captured_at: self.captured_at,
            window_title: None,
        };
        action::run(platform, &screenshot, &self.request)
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
