//! 文件系统：“图片”文件夹位置 + 标准库文件读写。

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use bps_platform::{FileSystem, NewFile, PlatformError};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::error::win_error;

/// Windows 文件系统。
#[derive(Debug, Default)]
pub struct WindowsFileSystem;

impl WindowsFileSystem {
    pub fn new() -> Self {
        Self
    }
}

impl FileSystem for WindowsFileSystem {
    fn pictures_dir(&self) -> Result<PathBuf, PlatformError> {
        unsafe {
            let path = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None)
                .map_err(|e| win_error("读取“图片”文件夹的位置失败", e))?;
            let result = PathBuf::from(OsString::from_wide(path.as_wide()));
            CoTaskMemFree(Some(path.0 as *const _));
            Ok(result)
        }
    }

    fn create_dir_all(&mut self, path: &Path) -> Result<(), PlatformError> {
        fs::create_dir_all(path).map_err(|e| io_error("创建目录", path, e))
    }

    fn create_new(&mut self, path: &Path, contents: &[u8]) -> Result<NewFile, PlatformError> {
        let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Ok(NewFile::AlreadyExists);
            }
            Err(e) => return Err(io_error("创建文件", path, e)),
        };
        if let Err(e) = file.write_all(contents) {
            drop(file);
            // 不留下写了一半的文件；删除的是刚刚由本函数新建的文件。
            let _ = fs::remove_file(path);
            return Err(io_error("写入文件", path, e));
        }
        Ok(NewFile::Created)
    }
}

fn io_error(action: &str, path: &Path, error: io::Error) -> PlatformError {
    PlatformError::new(format!("{action}失败：{}：{error}", path.display()))
}
