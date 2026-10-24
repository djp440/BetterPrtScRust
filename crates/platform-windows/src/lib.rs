//! `bps-platform` 能力 trait 的 Windows 实现（ADR 0002），尽量薄。
//!
//! - [`DxgiFrameSource`]：显示器布局 + 鼠标位置 + DXGI Desktop Duplication 截图
//! - [`WindowsClipboard`]：按 `ClipboardItem` 写入 `PNG` / `CF_DIB`
//! - [`WindowsHotkeys`]：`RegisterHotKey`，按下时由 [`EventLoop`] 递交 `PlatformEvent`
//! - [`WindowsNotifier`]：托盘图标的气泡通知，点击时打开文件或文件夹
//! - [`WindowsConfigFile`]：配置文件读写，所在目录变化时由 [`EventLoop`] 递交 `PlatformEvent`
//! - [`EventLoop`]：常驻实例的隐藏窗口、消息循环和托盘图标；[`EventSender`] 供其他线程投递事件
//! - [`ResidentPipe`]：常驻实例的命名管道（单实例 + 接收转交的请求）；
//!   [`PipeChannel`]：CLI 一侧的 `ResidentChannel`
//!
//! 进程必须是 Per-Monitor V2 DPI 感知，坐标和图像才是物理像素，见 [`dpi`]。
//!
//! 真机冒烟测试在 `tests/smoke.rs`，默认忽略：
//! `cargo test -p bps-platform-windows -- --ignored`。
#![cfg(windows)]

mod clipboard;
mod config_file;
mod display_config;
pub mod dpi;
mod error;
mod event_loop;
mod frame_source;
mod hotkeys;
mod notifier;
mod pipe;
mod pixels;

pub use clipboard::WindowsClipboard;
pub use config_file::WindowsConfigFile;
pub use event_loop::{EventLoop, EventSender};
pub use frame_source::DxgiFrameSource;
pub use hotkeys::WindowsHotkeys;
pub use notifier::WindowsNotifier;
pub use pipe::{PipeChannel, PipeClaim, ResidentPipe, resident_pipe_name};

use std::path::PathBuf;

use bps_platform::Platform;

/// 以 `event_loop` 的隐藏窗口为宿主，创建整套 Windows 平台能力；配置文件位于 `config_file`。
pub fn platform(event_loop: &EventLoop, config_file: PathBuf) -> Platform {
    Platform {
        frame_source: Box::new(DxgiFrameSource::new()),
        clipboard: Box::new(WindowsClipboard::new(Some(event_loop))),
        hotkeys: Box::new(event_loop.hotkeys()),
        notifier: Box::new(event_loop.notifier()),
        config_file: Box::new(event_loop.config_file(config_file)),
    }
}
