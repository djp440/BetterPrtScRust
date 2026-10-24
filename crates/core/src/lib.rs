//! BetterPrtSc 平台无关的核心逻辑。
//!
//! 唯一入口是 [`Command`]：热键、CLI、托盘菜单等所有触发方式都先转换成命令，
//! 再交给 [`ResidentInstance::execute`] 执行。平台能力通过 `bps_platform::Platform`
//! 注入，测试时换成 `bps-platform-fake`。
//!
//! 热键绑定来自配置文件 [`Config`]：[`ResidentInstance::start`] 读取（首次运行时写入
//! [`DEFAULT_CONFIG`]）并注册热键，配置文件被外部修改后自动重新加载。

mod action;
mod binding;
mod capture;
mod command;
mod config;
mod encode;
mod error;
mod instance;
mod location;
mod output_format;

pub use binding::{HotkeyBinding, HotkeyRegistrationFailure, RegistrationError, default_bindings};
pub use command::{
    CaptureMode, CaptureOverrides, CaptureRequest, Command, Flow, PostCaptureAction,
};
pub use config::{Config, ConfigError, DEFAULT_CONFIG};
pub use error::CoreError;
pub use instance::ResidentInstance;
pub use location::{DataLocation, PORTABLE_MARKER};
pub use output_format::OutputFormat;
