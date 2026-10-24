use crate::PlatformError;

/// 独立窗口界面：放在按需启动、用完即退出的子进程里，不在常驻实例里加载界面库。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandaloneWindow {
    /// 设置界面。
    Settings,
}

/// 打开独立窗口界面。
pub trait WindowLauncher {
    /// 打开 `window`；它已经打开时把它带到前台，不会再开第二个。
    fn open(&mut self, window: StandaloneWindow) -> Result<(), PlatformError>;
}
