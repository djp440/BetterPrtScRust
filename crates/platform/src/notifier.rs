use std::path::PathBuf;

/// 发给用户的一条通知，例如“截图已保存”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// 用户点击通知时执行的操作；`None` 表示点击后什么也不做。
    pub on_click: Option<NotificationClick>,
}

/// 点击通知时执行的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationClick {
    /// 用系统默认程序打开这个文件。
    OpenFile(PathBuf),
    /// 在文件管理器中打开这个文件夹。
    OpenFolder(PathBuf),
    /// 打开系统设置中控制“PrtSc 键交给系统截图工具”的页面
    /// （Windows：“键盘”设置页里的“使用 Print Screen 键打开屏幕截图”）。
    OpenPrintScreenKeySettings,
}

/// 通知能力（Windows 上是托盘气泡）。
///
/// 通知是尽力而为的：显示失败（例如托盘图标不存在、用户关闭了通知）不影响调用方。
pub trait Notifier {
    fn notify(&self, notification: Notification);
}
