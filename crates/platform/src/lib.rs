//! BetterPrtSc 的平台能力 trait（ADR 0002）。
//!
//! 这里只定义接口和跨平台共用的数据类型，不含任何实现：
//! - `bps-platform-windows` 是当前唯一的真实实现；
//! - `bps-platform-fake` 是测试用的假实现，`bps-core` 的测试通过它驱动。
//!
//! trait 按能力划分（截屏、热键、剪贴板……），不按某个平台的 API 划分。
//! 新增一种能力时：在这里加 trait，在 [`Platform`] 里加一个字段，
//! 然后分别在 `platform-windows` 和 `platform-fake` 里实现。

mod clipboard;
mod clock;
mod error;
mod event;
mod file_system;
mod frame_source;
mod geometry;
mod hotkey;
mod image;
mod notifier;

pub use clipboard::{Clipboard, ClipboardItem};
pub use clock::{Clock, LocalDateTime};
pub use error::PlatformError;
pub use event::PlatformEvent;
pub use file_system::{FileSystem, NewFile};
pub use frame_source::{Display, DisplayId, FrameSource};
pub use geometry::{Point, Rect};
pub use hotkey::{GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers};
pub use image::Image;
pub use notifier::{Notification, NotificationClick, Notifier};

/// 交给 `bps-core` 的一整套平台能力。
///
/// 每种能力一个字段；常驻实例持有它，在执行命令时调用。
pub struct Platform {
    /// 截屏能力（帧源）：显示器布局、鼠标位置、截取画面。
    pub frame_source: Box<dyn FrameSource>,
    /// 系统剪贴板。
    pub clipboard: Box<dyn Clipboard>,
    /// 全局热键的注册与注销；按下事件经平台事件循环以 [`PlatformEvent`] 递交。
    pub hotkeys: Box<dyn GlobalHotkeys>,
    /// 文件系统：自动存储写盘。
    pub file_system: Box<dyn FileSystem>,
    /// 通知（托盘气泡）。
    pub notifier: Box<dyn Notifier>,
    /// 本地时间，用于命名模板。
    pub clock: Box<dyn Clock>,
}
