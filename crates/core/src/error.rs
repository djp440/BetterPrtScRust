use std::fmt;

use bps_platform::PlatformError;

/// 执行命令失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// 平台能力调用失败。
    Platform(PlatformError),
    /// 帧源没有报告任何显示器。
    NoDisplay,
    /// 截图编码失败。
    Encode(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Platform(e) => write!(f, "平台调用失败：{e}"),
            CoreError::NoDisplay => f.write_str("没有可用的显示器"),
            CoreError::Encode(e) => write!(f, "截图编码失败：{e}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PlatformError> for CoreError {
    fn from(e: PlatformError) -> Self {
        CoreError::Platform(e)
    }
}
