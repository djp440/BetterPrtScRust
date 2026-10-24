use crate::{HotkeyId, IncomingRequest};

/// 平台事件循环递交给常驻实例的触发事件。
///
/// 事件本身不含业务含义，由 `core` 把它转换成命令再执行。
/// 请求事件带着回复通道，所以事件不能复制。
#[derive(Debug)]
pub enum PlatformEvent {
    /// 用 [`crate::GlobalHotkeys::register`] 注册的热键被按下。
    HotkeyPressed(HotkeyId),
    /// 用户或系统要求常驻实例退出，例如点击了托盘菜单的“退出”。
    ExitRequested,
    /// 其他进程（`bps` CLI、再次启动的程序）经 [`crate::ResidentChannel`] 转交来一条请求。
    RequestReceived(IncomingRequest),
}
