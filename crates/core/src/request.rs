//! 其他进程转交给常驻实例的请求、常驻实例的回复，以及它们在通道上的文本形式。
//!
//! 文本形式是 `bps` / 再次启动的程序 / 设置界面与常驻实例之间的约定：一条请求、一条回复，UTF-8。
//! - 请求：`capture <捕获模式名>`、`activate`、`status`
//! - 回复：`ok`、`cancelled`、`failed <说明>`，以及对 `status` 的状态回复
//!   （格式见 [`ResidentStatus`]）

use crate::{CaptureMode, ResidentStatus};

/// 其他进程转交给常驻实例的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// `bps capture <捕获模式>`：与按下该捕获模式的热键完全一致。
    Capture(CaptureMode),
    /// 再次启动程序时，激活已在运行的常驻实例（打开设置界面）。
    Activate,
    /// 设置界面查询常驻实例的状态（注册失败的热键绑定、检查更新的结果）；只读，不执行命令。
    Status,
}

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
        }
    }

    /// 解析请求文本；无法识别时返回 `None`。
    pub fn decode(message: &str) -> Option<Request> {
        match message.split_once(' ') {
            Some(("capture", name)) => mode_from_name(name).map(Request::Capture),
            None if message == "activate" => Some(Request::Activate),
            None if message == "status" => Some(Request::Status),
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
pub(crate) const MODE_NAMES: [(CaptureMode, &str); 4] = [
    (CaptureMode::Display, "display"),
    (CaptureMode::VirtualDesktop, "virtual-desktop"),
    (CaptureMode::Region, "region"),
    (CaptureMode::Window, "window"),
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
