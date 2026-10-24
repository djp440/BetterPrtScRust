use crate::PlatformError;

/// 系统自带的截图工具（Windows：“截图工具”）对 PrtSc 键的占用。
///
/// 只读取系统设置，从不修改：要解除占用，由用户自己去系统设置里关闭
/// （点击 [`crate::NotificationClick::OpenPrintScreenKeySettings`] 通知可直达设置页面）。
pub trait SystemScreenshotTool {
    /// 系统设置是否把 PrtSc 键交给了系统自带的截图工具
    /// （Windows：“使用 Print Screen 键打开屏幕截图”）。
    fn takes_print_screen_key(&self) -> Result<bool, PlatformError>;

    /// 开始监视这项系统设置：用户修改后，平台事件循环递交
    /// [`crate::PlatformEvent::PrintScreenKeySettingChanged`]。
    fn watch(&mut self) -> Result<(), PlatformError>;
}
