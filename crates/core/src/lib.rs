//! BetterPrtSc 平台无关的核心逻辑。
//!
//! 唯一入口是 [`Command`]：热键、CLI、托盘菜单等所有触发方式都先转换成命令，
//! 再交给 [`ResidentInstance::execute`] 执行。平台能力通过 `bps_platform::Platform`
//! 注入，测试时换成 `bps-platform-fake`。
//!
//! 其他进程（`bps` CLI、再次启动的程序）用 [`forward`] / [`run_cli`] 把 [`Request`]
//! 经 `bps_platform::ResidentChannel` 转交给常驻实例；常驻实例收到的请求事件由
//! [`ResidentInstance::handle`] 转换成命令执行，并回复 [`Reply`]。
//!
//! 跨显示器拼接以公共函数提供，供虚拟桌面与区域捕获共用：[`capture_rect`] 按虚拟桌面坐标矩形
//! 截图，[`stitch`] 把已截下的各显示器画面（[`DesktopImage`]）按位置拼接并裁剪。

mod action;
mod binding;
mod capture;
mod cli;
mod command;
mod encode;
mod error;
mod forward;
mod instance;
mod request;
mod stitch;

pub use binding::{HotkeyBinding, HotkeyRegistrationFailure, default_bindings};
pub use cli::{CliOutcome, CliStatus, run_cli};
pub use command::{CaptureMode, CaptureRequest, Command, Flow, PostCaptureAction};
pub use error::CoreError;
pub use forward::{ForwardError, WhenNotRunning, forward};
pub use instance::ResidentInstance;
pub use request::{Reply, Request};
pub use stitch::{DesktopImage, capture_rect, stitch, virtual_desktop_bounds};
