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
mod config_file;
mod error;
mod event;
mod frame_source;
mod geometry;
mod hotkey;
mod image;
mod notifier;

pub use clipboard::{Clipboard, ClipboardItem};
pub use config_file::ConfigFile;
pub use error::PlatformError;
pub use event::PlatformEvent;
pub use frame_source::{Display, DisplayId, FrameSource};
pub use geometry::{Point, Rect};
pub use hotkey::{GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers, ParseHotkeyError};
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
    /// 通知（Windows 上是托盘气泡）。
    pub notifier: Box<dyn Notifier>,
    /// 配置文件的读写与变更监视；变更经平台事件循环以 [`PlatformEvent`] 递交。
    pub config_file: Box<dyn ConfigFile>,
}
