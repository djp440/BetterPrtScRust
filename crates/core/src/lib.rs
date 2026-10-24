//! BetterPrtSc 平台无关的核心逻辑。
//!
//! 唯一入口是 [`Command`]：热键、CLI、托盘菜单等所有触发方式都先转换成命令，
//! 再交给 [`ResidentInstance::execute`] 执行。平台能力通过 `bps_platform::Platform`
//! 注入，测试时换成 `bps-platform-fake`。
//!
//! 热键绑定来自配置文件 [`Config`]：[`ResidentInstance::start`] 读取（首次运行时写入
//! [`DEFAULT_CONFIG`]）并注册热键，配置文件被外部修改后自动重新加载。
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
mod config;
mod encode;
mod error;
mod filename;
mod forward;
mod i18n;
mod instance;
mod location;
mod output;
mod request;
mod settings;
mod status;
mod stitch;
mod update;

pub use binding::{HotkeyBinding, HotkeyRegistrationFailure, RegistrationError, default_bindings};
pub use cli::{CliOutcome, CliStatus, run_cli};
pub use command::{
    CaptureMode, CaptureRequest, Command, Flow, PostCaptureAction, RecordingFormat,
    RecordingOptions,
};
pub use config::{Config, ConfigError, DEFAULT_CONFIG};
pub use error::CoreError;
pub use filename::{
    DEFAULT_FILENAME_TEMPLATE, FilenameFields, FilenameTemplate, FilenameTemplateError,
};
pub use forward::{ForwardError, WhenNotRunning, forward};
pub use i18n::{Language, Locale, Texts, UnknownLanguage};
pub use instance::ResidentInstance;
pub use location::{DataLocation, PORTABLE_MARKER};
pub use output::{OutputFormat, OutputOptions, SaveDir};
pub use request::{Reply, Request};
pub use settings::{BindingSettings, Settings, SettingsDocument, SettingsProblem};
pub use status::{BindingFailure, ResidentStatus, UpdateStatus};
pub use stitch::{DesktopImage, capture_rect, stitch, virtual_desktop_bounds};
pub use update::VERSION;
