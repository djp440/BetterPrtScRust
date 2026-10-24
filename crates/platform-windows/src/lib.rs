//! `bps-platform` 能力 trait 的 Windows 实现（ADR 0002），尽量薄。
//!
//! - [`DxgiFrameSource`]：显示器布局 + 鼠标位置 + DXGI Desktop Duplication 截图
//! - [`WindowsClipboard`]：按 `ClipboardItem` 写入 `PNG` / `JFIF` / `CF_DIB` / `CF_HDROP` 等
//! - [`WindowsHotkeys`]：`RegisterHotKey`，按下时由 [`EventLoop`] 递交 `PlatformEvent`
//! - [`WindowsFileSystem`]：“图片”文件夹位置 + 不覆盖已有文件的写盘
//! - [`TrayNotifier`]：托盘气泡通知，点击后由 [`EventLoop`] 打开文件或文件夹
//! - [`WindowsClock`]：本地时间
//! - [`WindowsConfigFile`]：配置文件读写，所在目录变化时由 [`EventLoop`] 递交 `PlatformEvent`
//! - [`WindowsAutostart`]：开机自启（`HKCU\...\Run` 项，或以最高权限运行的计划任务）
//! - [`WindowsElevation`]：是否以管理员权限运行、以管理员身份重新启动
//! - [`WindowsScreenshotTool`]：读取系统“截图工具”是否占用 PrtSc 键（只读），设置变化时由
//!   [`EventLoop`] 递交 `PlatformEvent`
//! - [`D2dSelectionOverlay`]：选区遮罩（Win32 + D3D11 + Direct2D，设备与窗口常驻预热），用户输入由 [`EventLoop`] 递交
//! - [`WgcVideoSource`]：录屏的视频源（Windows.Graphics.Capture，按区域裁剪，GPU 纹理）
//! - [`MfVideoEncoder`]：录屏的 MP4 编码器（Media Foundation，优先硬件 H.264 编码）
//! - [`WindowsTicker`]：录屏的节拍（高精度可等待定时器），由 [`EventLoop`] 递交
//! - [`EventLoop`]：常驻实例的隐藏窗口、消息循环和托盘图标；[`EventSender`] 供其他线程投递事件；
//!   [`WindowsTrayMenu`]：托盘菜单的内容
//! - [`ChildProcessWindows`]：以子进程（`BetterPrtSc.exe --settings` 等）打开独立窗口界面
//! - [`WindowsSystemLanguage`]：系统界面语言
//! - [`GitHubReleases`]：检查更新（WinHTTP 查询 GitHub Releases，只在用户开启时使用）
//! - [`map_system_font`]：给界面子进程只读映射系统字体；[`open_with_default_program`]、
//!   [`pick_folder`]、[`standard_user_can_write`]：界面子进程用的外壳操作
//! - [`ResidentPipe`]：常驻实例的命名管道（单实例 + 接收转交的请求）；
//!   [`PipeChannel`]：CLI 一侧的 `ResidentChannel`
//!
//! 进程必须是 Per-Monitor V2 DPI 感知，坐标和图像才是物理像素，见 [`dpi`]。
//!
//! 真机冒烟测试在 `tests/smoke.rs`，默认忽略：
//! `cargo test -p bps-platform-windows -- --ignored`。
#![cfg(windows)]

mod autostart;
mod child_windows;
mod clipboard;
mod clock;
mod config_file;
mod display_config;
pub mod dpi;
mod error;
mod event_loop;
mod file_system;
mod font;
mod frame_source;
mod hotkeys;
mod language;
mod notifier;
mod overlay;
mod pipe;
mod pixels;
mod registry;
mod screenshot_tool;
mod shell;
mod ticker;
mod token;
mod update;
mod video;

pub use autostart::WindowsAutostart;
pub use child_windows::{ChildProcessWindows, window_argument};
pub use clipboard::WindowsClipboard;
pub use clock::WindowsClock;
pub use config_file::WindowsConfigFile;
pub use event_loop::{EventLoop, EventSender, WindowsTrayMenu};
pub use file_system::WindowsFileSystem;
pub use font::map_system_font;
pub use frame_source::DxgiFrameSource;
pub use hotkeys::WindowsHotkeys;
pub use language::{WindowsSystemLanguage, user_ui_languages};
pub use notifier::TrayNotifier;
pub use overlay::D2dSelectionOverlay;
pub use pipe::{PipeChannel, PipeClaim, ResidentPipe, resident_pipe_name};
pub use screenshot_tool::WindowsScreenshotTool;
pub use shell::{
    allow_any_process_to_set_foreground, open_with_default_program, pick_folder,
    standard_user_can_write,
};
pub use ticker::WindowsTicker;
pub use token::{
    RESTARTED_AS_ADMINISTRATOR_ARG, WindowsElevation, is_elevated,
    run_self_as_administrator_and_wait,
};
pub use update::{GitHubReleases, latest_release};
pub use video::{MfVideoEncoder, WgcVideoSource};

use std::path::PathBuf;

use bps_platform::Platform;

/// 以 `event_loop` 的隐藏窗口为宿主，创建整套 Windows 平台能力；配置文件位于 `config_file`，
/// 设置界面等独立窗口由 `ui_exe`（通常就是 `BetterPrtSc.exe` 自己）以子进程打开。
pub fn platform(event_loop: &EventLoop, config_file: PathBuf, ui_exe: PathBuf) -> Platform {
    Platform {
        frame_source: Box::new(DxgiFrameSource::new()),
        clipboard: Box::new(WindowsClipboard::new(Some(event_loop))),
        hotkeys: Box::new(event_loop.hotkeys()),
        file_system: Box::new(WindowsFileSystem::new()),
        notifier: Box::new(event_loop.notifier()),
        clock: Box::new(WindowsClock::new()),
        config_file: Box::new(event_loop.config_file(config_file)),
        tray_menu: Box::new(event_loop.tray_menu()),
        windows: Box::new(ChildProcessWindows::new(ui_exe)),
        system_language: Box::new(WindowsSystemLanguage),
        updates: Box::new(GitHubReleases::new(event_loop.sender())),
        autostart: Box::new(WindowsAutostart::new()),
        elevation: Box::new(WindowsElevation::new()),
        system_screenshot_tool: Box::new(event_loop.system_screenshot_tool()),
        overlay: Box::new(event_loop.selection_overlay()),
        video_source: Box::new(WgcVideoSource::new()),
        video_encoder: Box::new(MfVideoEncoder::new()),
        ticker: Box::new(event_loop.ticker()),
    }
}
