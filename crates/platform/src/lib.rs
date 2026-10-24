//! BetterPrtSc 的平台能力 trait（ADR 0002）。
//!
//! 这里只定义接口和跨平台共用的数据类型，不含任何实现：
//! - `bps-platform-windows` 是当前唯一的真实实现；
//! - `bps-platform-fake` 是测试用的假实现，`bps-core` 的测试通过它驱动。
//!
//! trait 按能力划分（截屏、热键、剪贴板……），不按某个平台的 API 划分。
//! 新增一种能力时：在这里加 trait，在 [`Platform`] 里加一个字段，
//! 然后分别在 `platform-windows` 和 `platform-fake` 里实现。

mod autostart;
mod clipboard;
mod clock;
mod config_file;
mod error;
mod event;
mod file_dialog;
mod file_system;
mod frame_source;
mod geometry;
mod hotkey;
mod image;
mod language;
mod notifier;
mod overlay;
mod region_sampler;
mod request;
mod resident_channel;
mod screenshot_tool;
mod tray;
mod update;
mod video;
mod window;
mod window_source;

pub use autostart::{Autostart, AutostartEntry, AutostartMethod, Elevation};
pub use clipboard::{Clipboard, ClipboardItem};
pub use clock::{Clock, LocalDateTime};
pub use config_file::ConfigFile;
pub use error::PlatformError;
pub use event::PlatformEvent;
pub use file_dialog::{FileDialog, FileType, SaveAsOptions};
pub use file_system::{FileSystem, NewFile};
pub use frame_source::{Display, DisplayId, FrameSource};
pub use geometry::{Point, Rect};
pub use hotkey::{
    GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers, ParseHotkeyError, RegisterHotkeyError,
};
pub use image::Image;
pub use language::SystemLanguage;
pub use notifier::{Notification, NotificationClick, Notifier};
pub use overlay::{
    Backdrop, Color, Corner, Label, OverlayCursor, OverlayInput, OverlayItem, OverlayKey,
    OverlayScene, PointerButton, SelectionOverlay,
};
pub use region_sampler::RegionSampler;
pub use request::IncomingRequest;
pub use resident_channel::{ChannelError, ResidentChannel};
pub use screenshot_tool::SystemScreenshotTool;
pub use tray::{TrayMenu, TrayMenuItem, TrayMenuItemId};
pub use update::{LatestRelease, UpdateService};
pub use video::{
    StreamFrame, StreamOptions, Ticker, VideoEncoder, VideoFormat, VideoFrame, VideoSettings,
    VideoSource, VideoStream, VideoTarget, VideoWriter,
};
pub use window::{StandaloneWindow, WindowLauncher};
pub use window_source::{TopLevelWindow, WindowId, WindowLayout, WindowSource};

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
    /// 本地时间（用于命名模板）与单调时间（用于计量时长）。
    pub clock: Box<dyn Clock>,
    /// 配置文件的读写与变更监视；变更经平台事件循环以 [`PlatformEvent`] 递交。
    pub config_file: Box<dyn ConfigFile>,
    /// 开机自启的登记与移除。
    pub autostart: Box<dyn Autostart>,
    /// 本进程的权限，以及以管理员身份重新启动。
    pub elevation: Box<dyn Elevation>,
    /// 系统自带截图工具对 PrtSc 键的占用（只读）；设置变化经平台事件循环以 [`PlatformEvent`] 递交。
    pub system_screenshot_tool: Box<dyn SystemScreenshotTool>,
    /// 托盘图标的右键菜单；选中某项经平台事件循环以 [`PlatformEvent`] 递交。
    pub tray_menu: Box<dyn TrayMenu>,
    /// 打开设置界面等独立窗口（各在一个按需启动的子进程里）。
    pub windows: Box<dyn WindowLauncher>,
    /// 系统的界面语言，用于“跟随系统”的界面语言。
    pub system_language: Box<dyn SystemLanguage>,
    /// 检查更新；结果经平台事件循环以 [`PlatformEvent`] 递交。
    pub updates: Box<dyn UpdateService>,
    /// 选区遮罩：拍冻结帧、按 `core` 给出的场景绘制，用户输入以 [`PlatformEvent::OverlayInput`] 递交。
    pub overlay: Box<dyn SelectionOverlay>,
    /// 录屏的视频源：持续捕获屏幕上的一块区域。
    pub video_source: Box<dyn VideoSource>,
    /// 把录屏的帧编码成视频文件。
    pub video_encoder: Box<dyn VideoEncoder>,
    /// 节拍：录屏期间按帧率递交 [`PlatformEvent::Tick`]。
    pub ticker: Box<dyn Ticker>,
    /// 文件对话框（另存为）。
    pub file_dialog: Box<dyn FileDialog>,
    /// 窗口枚举与命中测试，以及截取单个窗口的完整内容。
    pub window_source: Box<dyn WindowSource>,
    /// 长截屏期间持续采样区域的实时画面；有新画面时经平台事件循环以 [`PlatformEvent`] 递交。
    pub region_sampler: Box<dyn RegionSampler>,
}
