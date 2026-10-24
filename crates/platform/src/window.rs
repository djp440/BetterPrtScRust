use crate::{Image, PlatformError};

/// 独立窗口界面：放在按需启动、用完即退出的子进程里，不在常驻实例里加载界面库。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandaloneWindow {
    /// 设置界面。同时只开一个：已经打开时把它带到前台。
    Settings,
    /// 结果面板：在本机识别 `screenshot` 里的文字（OCR）并显示，文字可以编辑和复制。
    /// 每次 OCR 都打开一个新的结果面板。
    ResultPanel {
        /// 要识别文字的截图。
        screenshot: Image,
    },
}

/// 打开独立窗口界面。
pub trait WindowLauncher {
    /// 打开 `window`。设置界面已经打开时把它带到前台，不会再开第二个。
    fn open(&mut self, window: StandaloneWindow) -> Result<(), PlatformError>;
}
