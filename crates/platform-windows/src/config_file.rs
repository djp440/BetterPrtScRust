//! 配置文件：普通文件读写 + 所在目录的变更通知。

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::error::{wide_path, win_error};
use crate::event_loop::Shared;
use bps_platform::{ConfigFile, PlatformError};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FindCloseChangeNotification, FindFirstChangeNotificationW,
};

/// 位于 `path` 的配置文件。监视由创建它的事件循环负责：目录有变化时，
/// 事件循环稍等片刻（合并编辑器保存时的多次写入）再递交 `ConfigFileChanged`。
pub struct WindowsConfigFile {
    path: PathBuf,
    shared: Rc<Shared>,
}

impl WindowsConfigFile {
    pub(crate) fn new(path: PathBuf, shared: Rc<Shared>) -> Self {
        Self { path, shared }
    }

    fn error(&self, action: &str, e: std::io::Error) -> PlatformError {
        PlatformError::new(format!("{action} {} 失败：{e}", self.path.display()))
    }
}

impl ConfigFile for WindowsConfigFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<Option<String>, PlatformError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == ErrorKind::InvalidData => Err(PlatformError::new(format!(
                "{} 不是 UTF-8 编码的文本",
                self.path.display()
            ))),
            Err(e) => Err(self.error("读取", e)),
        }
    }

    fn write(&mut self, text: &str) -> Result<(), PlatformError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.error("创建目录", e))?;
        }
        // 先写临时文件再替换，避免监视方读到写了一半的内容。
        let temporary = self.path.with_extension("toml.tmp");
        std::fs::write(&temporary, text).map_err(|e| self.error("写入", e))?;
        std::fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            self.error("写入", e)
        })
    }

    fn watch(&mut self) -> Result<(), PlatformError> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| PlatformError::new("配置文件路径没有所在目录"))?;
        let handle = unsafe {
            FindFirstChangeNotificationW(
                &wide_path(dir),
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE,
            )
        }
        .map_err(|e| win_error(&format!("监视 {} 失败", dir.display()), e))?;
        *self.shared.config_watch.borrow_mut() = Some(ChangeNotification(handle));
        Ok(())
    }
}

/// 目录变更通知句柄，被丢弃时关闭。
pub(crate) struct ChangeNotification(pub HANDLE);

impl Drop for ChangeNotification {
    fn drop(&mut self) {
        let _ = unsafe { FindCloseChangeNotification(self.0) };
    }
}
