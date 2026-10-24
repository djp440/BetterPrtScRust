//! BetterPrtSc 平台无关的核心逻辑。
//!
//! 唯一入口是 [`Command`]：热键、CLI、托盘菜单等所有触发方式都先转换成命令，
//! 再交给 [`ResidentInstance::execute`] 执行。平台能力通过 `bps_platform::Platform`
//! 注入，测试时换成 `bps-platform-fake`。

mod action;
mod binding;
mod capture;
mod command;
mod encode;
mod error;
mod filename;
mod instance;
mod output;

pub use binding::{HotkeyBinding, HotkeyRegistrationFailure, default_bindings};
pub use command::{CaptureMode, CaptureRequest, Command, Flow, PostCaptureAction};
pub use error::CoreError;
pub use filename::{
    DEFAULT_FILENAME_TEMPLATE, FilenameFields, FilenameTemplate, FilenameTemplateError,
};
pub use instance::ResidentInstance;
pub use output::{OutputFormat, OutputOptions, SaveDir};
