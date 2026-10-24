use std::sync::{Arc, Mutex};

use bps_platform::{PlatformError, RecordingToolbar, RecordingToolbarView};

/// 假录屏控制条：只记下最近一次显示的样子；隐藏后为 `None`。
pub(crate) struct FakeRecordingToolbar(pub Arc<Mutex<Option<RecordingToolbarView>>>);

impl RecordingToolbar for FakeRecordingToolbar {
    fn show(&mut self, view: &RecordingToolbarView) -> Result<(), PlatformError> {
        *self.0.lock().unwrap() = Some(view.clone());
        Ok(())
    }

    fn hide(&mut self) {
        *self.0.lock().unwrap() = None;
    }
}
