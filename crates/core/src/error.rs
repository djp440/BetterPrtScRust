use std::fmt;

use bps_platform::PlatformError;

use crate::{CaptureMode, PostCaptureAction};

/// 执行命令失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// 平台能力调用失败。
    Platform(PlatformError),
    /// 帧源没有报告任何显示器。
    NoDisplay,
    /// 截图编码失败。
    Encode(String),
    /// 这个捕获模式尚未实现；捕获没有开始，不产生任何产物。
    UnsupportedCaptureMode(CaptureMode),
    /// 这个截后动作尚未实现；捕获没有开始，不产生任何产物。
    UnsupportedAction(PostCaptureAction),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Platform(e) => write!(f, "平台调用失败：{e}"),
            CoreError::NoDisplay => f.write_str("没有可用的显示器"),
            CoreError::Encode(e) => write!(f, "截图编码失败：{e}"),
            CoreError::UnsupportedCaptureMode(mode) => write!(f, "暂不支持“{mode}”捕获模式"),
            CoreError::UnsupportedAction(action) => write!(f, "暂不支持“{action}”截后动作"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PlatformError> for CoreError {
    fn from(e: PlatformError) -> Self {
        CoreError::Platform(e)
    }
}
