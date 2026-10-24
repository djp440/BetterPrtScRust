use std::sync::{Arc, Mutex};

use bps_platform::{PlatformError, StandaloneWindow, WindowLauncher};

#[derive(Default)]
pub(crate) struct WindowsState {
    /// 按打开顺序记下的独立窗口。
    pub opened: Vec<StandaloneWindow>,
    /// 打开窗口是否失败（例如找不到程序文件）。
    pub fails: bool,
}

pub(crate) struct FakeWindowLauncher(pub Arc<Mutex<WindowsState>>);

impl WindowLauncher for FakeWindowLauncher {
    fn open(&mut self, window: StandaloneWindow) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        if state.fails {
            return Err(PlatformError::new("找不到程序文件"));
        }
        state.opened.push(window);
        Ok(())
    }
}
