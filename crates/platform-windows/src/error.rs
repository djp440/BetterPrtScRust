use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use bps_platform::PlatformError;
use windows::core::HSTRING;

/// 把 Win32 / COM 错误包装成带上下文的平台错误。
pub(crate) fn win_error(context: &str, error: windows::core::Error) -> PlatformError {
    PlatformError::new(format!("{context}：{error}（{:#010X}）", error.code().0))
}

/// 把路径转换成 Win32 API 使用的宽字符串（不经过 UTF-8，保留任意文件名）。
pub(crate) fn wide_path(path: &Path) -> HSTRING {
    HSTRING::from_wide(&path.as_os_str().encode_wide().collect::<Vec<u16>>())
}
