//! 常驻实例当前进行中的捕获：同一时刻最多一个，四种会话共用一套推进和收尾的入口。

use bps_platform::{IncomingRequest, OverlayInput, Platform};

use crate::capture::CaptureFailure;
use crate::long_screenshot::LongScreenshotCapture;
use crate::recording::{self, Recording};
use crate::region::{Ending, RegionCapture};
use crate::window::WindowCapture;

/// 进行中的、需要用户在遮罩上操作的捕获（区域、窗口、长截屏、录屏）。
pub(crate) enum ActiveCapture {
    /// 区域捕获，以及其他要在选区遮罩上标注的捕获。
    Region(Box<RegionCapture>),
    /// 窗口捕获。
    Window(WindowCapture),
    /// 长截屏。
    LongScreenshot(LongScreenshotCapture),
    /// 录屏（框选区域或正在录制）。
    Recording(Recording),
}

impl ActiveCapture {
    /// 记下经 CLI 转交来、等捕获结束再回复的请求。
    pub fn set_requester(&mut self, requester: IncomingRequest) {
        let slot = match self {
            ActiveCapture::Region(capture) => &mut capture.requester,
            ActiveCapture::Window(capture) => &mut capture.requester,
            ActiveCapture::LongScreenshot(capture) => &mut capture.requester,
            ActiveCapture::Recording(recording) => &mut recording.requester,
        };
        *slot = Some(requester);
    }

    /// 取走等待回复的请求（捕获结束时回复它）。
    pub fn into_requester(self) -> Option<IncomingRequest> {
        match self {
            ActiveCapture::Region(capture) => capture.requester,
            ActiveCapture::Window(capture) => capture.requester,
            ActiveCapture::LongScreenshot(capture) => capture.requester,
            ActiveCapture::Recording(recording) => recording.requester,
        }
    }

    /// 遮罩上的一次输入。返回 `None` 表示捕获还在进行。
    pub fn on_overlay_input(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self {
            ActiveCapture::Region(capture) => capture.handle(platform, input),
            ActiveCapture::Window(capture) => capture.handle(platform, input),
            ActiveCapture::LongScreenshot(capture) => capture.handle(platform, input),
            ActiveCapture::Recording(recording) => {
                recording.on_overlay_input(platform, input).map(to_ending)
            }
        }
    }

    /// 节拍：驱动录屏和长截屏的采样（其他捕获不关心节拍）。
    pub fn on_tick(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        match self {
            ActiveCapture::Recording(recording) => recording.on_tick(platform).map(to_ending),
            ActiveCapture::LongScreenshot(capture) => capture.on_sample(platform),
            ActiveCapture::Region(_) | ActiveCapture::Window(_) => None,
        }
    }

    /// 平台采到了长截屏的新画面。
    pub fn on_region_sampled(
        &mut self,
        platform: &mut Platform,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self {
            ActiveCapture::LongScreenshot(capture) => capture.on_sample(platform),
            _ => None,
        }
    }

    /// 停止录屏：正在录制时存储已录下的内容，还在框选时取消。其他捕获什么也不做。
    pub fn stop_recording(
        &mut self,
        platform: &mut Platform,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self {
            ActiveCapture::Recording(recording) => Some(to_ending(recording.stop(platform))),
            _ => None,
        }
    }

    /// 放弃这次捕获（例如退出程序）：停止采样、关闭遮罩。录屏不在这里处理（它要先存好已录下的内容）。
    pub fn abandon(&mut self, platform: &mut Platform) {
        match self {
            ActiveCapture::LongScreenshot(capture) => capture.abandon(platform),
            ActiveCapture::Region(_) | ActiveCapture::Window(_) | ActiveCapture::Recording(_) => {
                platform.overlay.close()
            }
        }
    }
}

fn to_ending(ending: Result<recording::Ending, CaptureFailure>) -> Result<Ending, CaptureFailure> {
    ending.map(|ending| match ending {
        recording::Ending::Saved => Ending::Delivered,
        recording::Ending::Cancelled => Ending::Cancelled,
    })
}
