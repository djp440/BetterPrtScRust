use std::sync::{Arc, Mutex};

use bps_platform::{
    Backdrop, Display, DisplayId, Image, OverlayScene, PlatformError, SelectionOverlay,
};

use crate::frame_source::FrameState;

/// 假遮罩的状态：是否打开、最近一次画出的场景、打开时拍下的冻结帧。
#[derive(Default)]
pub(crate) struct OverlayState {
    open: bool,
    /// 最近一次 `render` 的场景；打开后还没画过时为 `None`（遮罩还看不见）。
    scene: Option<OverlayScene>,
    frozen: Vec<(DisplayId, Image)>,
}

impl OverlayState {
    /// 遮罩是否正显示在屏幕上（打开并且至少画过一次）。
    pub fn visible_scene(&self) -> Option<&OverlayScene> {
        self.scene.as_ref().filter(|_| self.open)
    }
}

pub(crate) struct FakeOverlay {
    pub frames: Arc<Mutex<FrameState>>,
    pub state: Arc<Mutex<OverlayState>>,
}

impl SelectionOverlay for FakeOverlay {
    fn open(&mut self, backdrop: Backdrop) -> Result<Vec<Display>, PlatformError> {
        let Backdrop::FrozenFrame = backdrop;
        let mut state = self.state.lock().unwrap();
        if state.open {
            return Err(PlatformError::new("假遮罩已经打开"));
        }
        let frames = self.frames.lock().unwrap();
        let displays = frames.describe();
        if displays.is_empty() {
            return Err(PlatformError::new("假帧源中没有显示器"));
        }
        // 冻结帧：此刻每块显示器画面的副本，之后画面再变也不影响它。
        state.frozen = frames
            .displays
            .iter()
            .zip(&displays)
            .map(|(fake, display)| (display.id, fake.image().clone()))
            .collect();
        state.open = true;
        state.scene = None;
        Ok(displays)
    }

    fn render(&mut self, scene: &OverlayScene) -> Result<(), PlatformError> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        state.scene = Some(scene.clone());
        Ok(())
    }

    fn frozen_image(&self, display: DisplayId) -> Result<Image, PlatformError> {
        let state = self.state.lock().unwrap();
        state
            .frozen
            .iter()
            .find(|(id, _)| *id == display)
            .map(|(_, image)| image.clone())
            .ok_or_else(|| PlatformError::new(format!("冻结帧中没有显示器 {display:?}")))
    }

    fn close(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        state.scene = None;
        state.frozen.clear();
    }
}
