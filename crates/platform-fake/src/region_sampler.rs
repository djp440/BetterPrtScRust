use std::sync::{Arc, Mutex};

use bps_platform::{Image, PlatformError, Rect, RegionSampler};

use crate::frame_source::FrameState;

/// 假区域采样的状态：正在采样的区域、采到还没取走的画面、至今开始采样过的区域。
#[derive(Default)]
pub(crate) struct SamplerState {
    pub region: Option<Rect>,
    pub pending: Option<Result<Image, PlatformError>>,
    pub started: Vec<Rect>,
}

impl SamplerState {
    /// 截下此刻桌面上正在采样的区域，放进待取的位置；没有在采样时返回 `false`。
    pub fn sample(&mut self, frames: &FrameState) -> bool {
        let Some(region) = self.region else {
            return false;
        };
        self.pending = Some(Ok(frames.live_region(region)));
        true
    }
}

pub(crate) struct FakeRegionSampler(pub Arc<Mutex<SamplerState>>);

impl RegionSampler for FakeRegionSampler {
    fn start(&mut self, region: Rect) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        state.region = Some(region);
        state.pending = None;
        state.started.push(region);
        Ok(())
    }

    fn take_sample(&mut self) -> Result<Option<Image>, PlatformError> {
        self.0.lock().unwrap().pending.take().transpose()
    }

    fn stop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.region = None;
        state.pending = None;
    }
}
