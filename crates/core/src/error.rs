use std::fmt;
use std::path::PathBuf;

use bps_platform::{Hotkey, PlatformError, Rect};

use crate::{CaptureMode, PostCaptureAction, RecordingFormat};

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
    /// 这种录屏格式尚未实现；捕获没有开始，不产生任何产物。
    UnsupportedRecordingFormat(RecordingFormat),
    /// 框选的区域太小，录不成视频（例如 MP4 的宽和高至少 2 像素）。
    RecordingRegionTooSmall(Rect),
    /// 停止录屏时还没有收到任何画面，没有可存储的内容。
    NothingRecorded,
    /// 正在录屏，这次捕获被拒绝；`stop_hotkey` 是停止录屏的热键（录屏由热键绑定开始时）。
    RecordingInProgress { stop_hotkey: Option<Hotkey> },
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
            CoreError::CaptureInProgress => f.write_str("上一次捕获还没有结束"),
            CoreError::UnsupportedRecordingFormat(format) => {
                let name = match format {
                    RecordingFormat::Mp4 => "MP4",
                    RecordingFormat::Gif => "GIF",
                };
                write!(f, "暂不支持“{name}”录屏格式")
            }
            CoreError::RecordingRegionTooSmall(region) => write!(
                f,
                "区域 {} × {} 太小，无法录屏（宽和高至少 2 像素）",
                region.width, region.height
            ),
            CoreError::NothingRecorded => f.write_str("没有录到任何画面"),
            CoreError::RecordingInProgress {
                stop_hotkey: Some(hotkey),
            } => write!(f, "正在录屏，按 {hotkey} 停止录制后再捕获"),
            CoreError::RecordingInProgress { stop_hotkey: None } => {
                f.write_str("正在录屏，停止录制后再捕获")
            }
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PlatformError> for CoreError {
    fn from(e: PlatformError) -> Self {
        CoreError::Platform(e)
    }
}
