//! 区域采样（长截屏）：Windows.Graphics.Capture 的区域视频流 + 节拍。
//!
//! 复用录屏的视频源（`WgcVideoSource`，遮罩窗口设了 `WDA_EXCLUDEFROMCAPTURE`，不会进画面）；
//! 节拍到点时事件循环递交 `PlatformEvent::Tick`，`core` 再来取最新一帧。画面没有变化时视频流
//! 返回同一个 `Rc`，这里据此跳过，只把真正变了的画面读回成 CPU 图像。

use std::rc::Rc;
use std::time::Duration;

use bps_platform::{
    Image, PlatformError, Rect, RegionSampler, StreamFrame, StreamOptions, Ticker, VideoFrame,
    VideoSource, VideoStream, VideoTarget,
};

use crate::{WgcVideoSource, WindowsTicker};

/// 采样的节拍周期：约 30 次 / 秒，足够跟上正常速度的滚动。
const PERIOD: Duration = Duration::from_millis(33);

pub struct WgcRegionSampler {
    ticker: WindowsTicker,
    source: WgcVideoSource,
    stream: Option<Box<dyn VideoStream>>,
    last: Option<Rc<dyn VideoFrame>>,
}

impl WgcRegionSampler {
    pub(crate) fn new(ticker: WindowsTicker) -> Self {
        Self {
            ticker,
            source: WgcVideoSource::new(),
            stream: None,
            last: None,
        }
    }
}

impl RegionSampler for WgcRegionSampler {
    fn start(&mut self, region: Rect) -> Result<(), PlatformError> {
        self.stop();
        let stream = self.source.open(
            VideoTarget::Region(region),
            StreamOptions { show_cursor: false },
        )?;
        self.ticker
            .start(PERIOD)
            .inspect_err(|_| self.stream = None)?;
        self.stream = Some(stream);
        Ok(())
    }

    fn take_sample(&mut self) -> Result<Option<Image>, PlatformError> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };
        match stream.latest()? {
            StreamFrame::Pending => Ok(None),
            StreamFrame::Ended => Err(PlatformError::new("采样的区域已经不存在（显示器变化了）")),
            StreamFrame::Frame(frame) => {
                if self
                    .last
                    .as_ref()
                    .is_some_and(|last| Rc::ptr_eq(last, &frame))
                {
                    return Ok(None);
                }
                let image = frame.to_image()?;
                self.last = Some(frame);
                Ok(Some(image))
            }
        }
    }

    fn stop(&mut self) {
        self.ticker.stop();
        self.stream = None;
        self.last = None;
    }
}
