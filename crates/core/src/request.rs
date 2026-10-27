//! 其他进程转交给常驻实例的请求、常驻实例的回复，以及它们在通道上的文本形式。
//!
//! 文本形式是 `bps` / 再次启动的程序 / 设置界面与常驻实例之间的约定：一条请求、一条回复，UTF-8。
//! - 请求：`capture <捕获模式名>`、`activate`、`status`、`pause-hotkeys`、`resume-hotkeys`、
//!   `ocr-copied <字数 | empty | no-language-pack | failed 原因>`
//! - 回复：`ok`、`cancelled`、`failed <说明>`，以及对 `status` 的状态回复
//!   （格式见 [`ResidentStatus`]）

use crate::{CaptureMode, OcrCopy, ResidentStatus};

/// 其他进程转交给常驻实例的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// `bps capture <捕获模式>`：与按下该捕获模式的热键完全一致。
    Capture(CaptureMode),
    /// 再次启动程序时，激活已在运行的常驻实例（打开设置界面）。
    Activate,
    /// 设置界面查询常驻实例的状态（注册失败的热键绑定、检查更新的结果）；只读，不执行命令。
    Status,
    /// 设置界面开始录制热键：暂停响应全局热键，免得按下录制的组合键时触发截图。
    /// 暂停是一份短租约（[`HOTKEY_PAUSE_LEASE`]），设置界面录制期间要不断续租；
    /// 设置进程异常退出没来得及恢复时，租约到期后热键自动恢复。
    PauseHotkeys,
    /// 录制结束：立即恢复响应全局热键。
    ResumeHotkeys,
    /// 不显示界面的结果面板子进程完成了“识别后自动复制”：常驻实例据此通知用户。
    OcrCopied(OcrCopy),
}

/// 暂停全局热键的租约时长：设置界面每秒续租一次，崩溃后最多这么久热键自动恢复。
pub const HOTKEY_PAUSE_LEASE: std::time::Duration = std::time::Duration::from_secs(4);

/// 常驻实例对一条请求的回复。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// 命令已执行完毕。
    Done,
    /// 用户取消了这次捕获（例如在选区遮罩上按了 Esc），没有任何产物。
    Cancelled,
    /// 命令执行失败，附带给人看的说明。
    Failed(String),
    /// 对 [`Request::Status`] 的回复。
    Status(ResidentStatus),
}

impl Request {
    /// 通道上的文本形式。
    pub fn encode(&self) -> String {
        match self {
            Request::Capture(mode) => format!("capture {}", mode_name(*mode)),
            Request::Activate => "activate".to_owned(),
            Request::Status => "status".to_owned(),
            Request::PauseHotkeys => "pause-hotkeys".to_owned(),
            Request::ResumeHotkeys => "resume-hotkeys".to_owned(),
            Request::OcrCopied(copy) => match copy {
                OcrCopy::Copied { characters } => format!("ocr-copied {characters}"),
                OcrCopy::Empty => "ocr-copied empty".to_owned(),
                OcrCopy::NoLanguagePack => "ocr-copied no-language-pack".to_owned(),
                // 原因单独占一行发送，里面的换行换成空格。
                OcrCopy::Failed(reason) => {
                    format!("ocr-copied failed {}", reason.replace(['\r', '\n'], " "))
                }
            },
        }
    }

    /// 解析请求文本；无法识别时返回 `None`。
    pub fn decode(message: &str) -> Option<Request> {
        match message.split_once(' ') {
            Some(("capture", name)) => mode_from_name(name).map(Request::Capture),
            Some(("ocr-copied", outcome)) => Some(Request::OcrCopied(match outcome {
                "empty" => OcrCopy::Empty,
                "no-language-pack" => OcrCopy::NoLanguagePack,
                "failed" => OcrCopy::Failed(String::new()),
                _ => match outcome.split_once(' ') {
                    Some(("failed", reason)) => OcrCopy::Failed(reason.to_owned()),
                    _ => OcrCopy::Copied {
                        characters: outcome.parse().ok()?,
                    },
                },
            })),
            None if message == "activate" => Some(Request::Activate),
            None if message == "status" => Some(Request::Status),
            None if message == "pause-hotkeys" => Some(Request::PauseHotkeys),
            None if message == "resume-hotkeys" => Some(Request::ResumeHotkeys),
            _ => None,
        }
    }
}

impl Reply {
    /// 通道上的文本形式。
    pub fn encode(&self) -> String {
        match self {
            Reply::Done => "ok".to_owned(),
            Reply::Cancelled => "cancelled".to_owned(),
            Reply::Failed(message) => format!("failed {message}"),
            Reply::Status(status) => status.encode(),
        }
    }

    /// 解析回复文本；无法识别的回复（例如版本不一致）当作失败。
    pub fn decode(message: &str) -> Reply {
        if let Some(status) = ResidentStatus::decode(message) {
            return Reply::Status(status);
        }
        match message.split_once(' ') {
            None if message == "ok" => Reply::Done,
            None if message == "cancelled" => Reply::Cancelled,
            Some(("failed", text)) => Reply::Failed(text.to_owned()),
            _ => Reply::Failed(format!("无法识别常驻实例的回复：{message}")),
        }
    }
}

/// 捕获模式在 CLI 参数和请求文本里的名称。
pub(crate) const MODE_NAMES: [(CaptureMode, &str); 3] = [
    (CaptureMode::Display, "display"),
    (CaptureMode::VirtualDesktop, "virtual-desktop"),
    (CaptureMode::Region, "region"),
];

pub(crate) fn mode_name(mode: CaptureMode) -> &'static str {
    MODE_NAMES
        .iter()
        .find(|(m, _)| *m == mode)
        .map(|(_, name)| *name)
        .expect("每种捕获模式都有名称")
}

pub(crate) fn mode_from_name(name: &str) -> Option<CaptureMode> {
    MODE_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(mode, _)| *mode)
}
