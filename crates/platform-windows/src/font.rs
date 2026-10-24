//! 给界面子进程用的系统字体：只读映射进内存，不复制。
//!
//! 中文字体文件很大（微软雅黑约 20 MB），映射后只有界面真正读到的页面进入工作集，
//! 而且这些页面是文件映射的共享页，不计入私有工作集。

use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, GENERIC_READ};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY,
};

use crate::error::wide_path;

/// 把 `%WINDIR%\Fonts\<file_name>` 只读映射进内存；文件不存在或映射失败时返回 `None`。
/// 映射在进程结束前一直有效。
pub fn map_system_font(file_name: &str) -> Option<&'static [u8]> {
    let windows_dir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let path = PathBuf::from(windows_dir).join("Fonts").join(file_name);
    unsafe {
        let file = CreateFileW(
            &wide_path(&path),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let mut size = 0i64;
        let mapped = GetFileSizeEx(file, &mut size)
            .ok()
            .filter(|_| size > 0)
            .and_then(|_| CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, None).ok());
        let _ = CloseHandle(file);
        let mapping = mapped?;
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        // 视图会让映射对象保持存在，句柄可以先关掉。
        let _ = CloseHandle(mapping);
        if view.Value.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(
            view.Value as *const u8,
            size as usize,
        ))
    }
}
