use std::path::PathBuf;

/// 发给用户的一条通知，例如热键注册失败、自动存储完成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// 用户点击通知时执行的操作；`None` 表示点击后只关闭通知。
    pub on_click: Option<NotificationClick>,
}

/// 点击通知后要打开的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationClick {
    /// 用关联程序打开文件。
    OpenFile(PathBuf),
    /// 在文件管理器中打开文件夹。
    OpenFolder(PathBuf),
}

/// 通知能力（Windows 上是托盘气泡）。
///
/// 通知是尽力而为的：发送失败（例如托盘图标不存在）时不报告错误。
pub trait Notifier {
    fn notify(&self, notification: Notification);
}
