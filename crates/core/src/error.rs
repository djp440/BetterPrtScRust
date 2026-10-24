use std::fmt;
use std::path::PathBuf;

use bps_platform::PlatformError;

use crate::{CaptureMode, Locale, PostCaptureAction, Texts};

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
    /// 上一次捕获还没有结束（例如选区遮罩还开着），这次捕获被拒绝。
    CaptureInProgress,
}

impl CoreError {
    /// 用 `texts` 的语言说明失败原因。
    pub fn localized(&self, texts: &Texts) -> String {
        match self {
            CoreError::Platform(e) => texts.format("error-platform", &[("error", e)]),
            CoreError::NoDisplay => texts.get("error-no-display"),
            CoreError::Encode(e) => texts.format("error-encode", &[("error", e)]),
            CoreError::TooManySameNameFiles(path) => texts.format(
                "error-too-many-same-name-files",
                &[("path", &path.display())],
            ),
            CoreError::UnsupportedCaptureMode(mode) => texts.format(
                "error-unsupported-capture-mode",
                &[("mode", &mode.localized_label(texts))],
            ),
            CoreError::UnsupportedAction(action) => texts.format(
                "error-unsupported-action",
                &[("action", &action.localized_label(texts))],
            ),
            CoreError::InvalidRequest(message) => {
                texts.format("error-invalid-request", &[("request", message)])
            }
            CoreError::CaptureInProgress => texts.get("error-capture-in-progress"),
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized(Texts::of(Locale::ZhCn)))
    }
}

impl std::error::Error for CoreError {}

impl From<PlatformError> for CoreError {
    fn from(e: PlatformError) -> Self {
        CoreError::Platform(e)
    }
}
