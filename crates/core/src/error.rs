use std::fmt;
use std::path::PathBuf;

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
    /// 存储目录里以 `-1`…`-N` 结尾的同名文件都已存在，找不到可用的文件名。
    TooManySameNameFiles(PathBuf),
    /// 这个捕获模式尚未实现；捕获没有开始，不产生任何产物。
    UnsupportedCaptureMode(CaptureMode),
    /// 这个截后动作尚未实现；捕获没有开始，不产生任何产物。
    UnsupportedAction(PostCaptureAction),
    /// 转交来的请求无法识别（例如 `bps` 与常驻实例的版本不一致），附带原始请求文本。
    InvalidRequest(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Platform(e) => write!(f, "平台调用失败：{e}"),
            CoreError::NoDisplay => f.write_str("没有可用的显示器"),
            CoreError::Encode(e) => write!(f, "截图编码失败：{e}"),
            CoreError::TooManySameNameFiles(path) => {
                write!(f, "同名文件过多，找不到可用的文件名：{}", path.display())
            }
            CoreError::UnsupportedCaptureMode(mode) => {
                write!(f, "暂不支持“{}”捕获模式", mode.label())
            }
            CoreError::UnsupportedAction(action) => {
                write!(f, "暂不支持“{}”截后动作", action.label())
            }
            CoreError::InvalidRequest(message) => write!(
                f,
                "常驻实例无法识别请求“{message}”，请确认 bps 与 BetterPrtSc 的版本一致"
            ),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PlatformError> for CoreError {
    fn from(e: PlatformError) -> Self {
        CoreError::Platform(e)
    }
}
