use std::fmt;

use crate::PlatformError;

/// CLI 一侧通往常驻实例的通道：把请求文本转交给常驻实例，并拉起没在运行的常驻实例。
///
/// 它不属于 [`crate::Platform`]（那是常驻实例自己的能力集合），而是由 `bps` CLI 和
/// 再次启动的 `BetterPrtSc.exe` 单独创建，交给 `core` 的转交逻辑使用。
/// 常驻实例一侧收到的请求以 [`crate::PlatformEvent::RequestReceived`] 递交。
pub trait ResidentChannel {
    /// 把一条请求交给常驻实例，并等待它的回复。
    fn send(&mut self, message: &str) -> Result<String, ChannelError>;

    /// 启动常驻实例，并等到它可以接收请求为止（或等到超时、进程提前退出）。
    fn launch_resident_instance(&mut self) -> Result<(), PlatformError>;
}

/// 转交请求失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    /// 常驻实例没在运行（通道的另一端不存在）。
    NotRunning,
    /// 连上了常驻实例，但请求或回复没能完成。
    Failed(PlatformError),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::NotRunning => f.write_str("常驻实例没在运行"),
            ChannelError::Failed(e) => write!(f, "与常驻实例通信失败：{e}"),
        }
    }
}

impl std::error::Error for ChannelError {}
