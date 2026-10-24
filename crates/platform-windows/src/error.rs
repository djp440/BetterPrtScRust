use bps_platform::PlatformError;

/// 把 Win32 / COM 错误包装成带上下文的平台错误。
pub(crate) fn win_error(context: &str, error: windows::core::Error) -> PlatformError {
    PlatformError::new(format!("{context}：{error}（{:#010X}）", error.code().0))
}
