use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bps_platform::{ConfigFile, PlatformError};

/// 假配置文件的位置；只用于提示和通知的点击操作，不会访问真实磁盘。
pub const FAKE_CONFIG_PATH: &str = r"C:\BetterPrtSc-fake\config.toml";

#[derive(Default)]
pub(crate) struct ConfigFileState {
    /// 文件内容；`None` 表示文件不存在。
    pub text: Option<String>,
    pub watched: bool,
}

pub(crate) struct FakeConfigFile {
    pub state: Arc<Mutex<ConfigFileState>>,
    pub path: PathBuf,
}

impl ConfigFile for FakeConfigFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<Option<String>, PlatformError> {
        Ok(self.state.lock().unwrap().text.clone())
    }

    fn write(&mut self, text: &str) -> Result<(), PlatformError> {
        self.state.lock().unwrap().text = Some(text.to_owned());
        Ok(())
    }

    fn watch(&mut self) -> Result<(), PlatformError> {
        self.state.lock().unwrap().watched = true;
        Ok(())
    }
}
