use std::fmt;

use bps_platform::{ChannelError, PlatformError, ResidentChannel};

use crate::{Reply, Request};

/// 转交请求时常驻实例没在运行怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenNotRunning {
    /// 先拉起常驻实例，再转交（`bps` CLI）。
    Launch,
    /// 直接报错（再次启动的程序只负责激活已在运行的实例，不再拉起新的）。
    GiveUp,
}

/// 把请求转交给常驻实例，返回它的回复。
pub fn forward(
    request: &Request,
    channel: &mut dyn ResidentChannel,
    when_not_running: WhenNotRunning,
) -> Result<Reply, ForwardError> {
    let message = request.encode();
    match channel.send(&message) {
        Ok(reply) => return Ok(Reply::decode(&reply)),
        Err(ChannelError::Failed(e)) => return Err(ForwardError::Channel(e)),
        Err(ChannelError::NotRunning) => {}
    }
    if when_not_running == WhenNotRunning::GiveUp {
        return Err(ForwardError::NotRunning);
    }
    channel
        .launch_resident_instance()
        .map_err(ForwardError::LaunchFailed)?;
    match channel.send(&message) {
        Ok(reply) => Ok(Reply::decode(&reply)),
        Err(ChannelError::Failed(e)) => Err(ForwardError::Channel(e)),
        Err(ChannelError::NotRunning) => Err(ForwardError::NotRunningAfterLaunch),
    }
}

/// 请求没能转交给常驻实例的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardError {
    /// 常驻实例没在运行。
    NotRunning,
    /// 拉起常驻实例失败。
    LaunchFailed(PlatformError),
    /// 拉起了常驻实例，但仍然连不上它。
    NotRunningAfterLaunch,
    /// 连上了常驻实例，但请求或回复没能完成。
    Channel(PlatformError),
}

impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForwardError::NotRunning => f.write_str("常驻实例没在运行"),
            ForwardError::LaunchFailed(e) => write!(f, "无法启动常驻实例：{e}"),
            ForwardError::NotRunningAfterLaunch => f.write_str("已启动常驻实例，但仍无法连接它"),
            ForwardError::Channel(e) => write!(f, "与常驻实例通信失败：{e}"),
        }
    }
}

impl std::error::Error for ForwardError {}
