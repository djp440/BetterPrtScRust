use std::fmt;

use bps_platform::{Hotkey, PlatformError};

use crate::{CaptureRequest, Config};

/// 热键绑定：一个全局热键对应一组“捕获模式 + 截后动作”，可带覆盖项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub hotkey: Hotkey,
    pub capture: CaptureRequest,
}

/// 内置的默认热键绑定，即 [`crate::DEFAULT_CONFIG`] 中的 6 条绑定。
pub fn default_bindings() -> Vec<HotkeyBinding> {
    Config::default().bindings
}

/// 一条热键绑定注册失败；设置界面据此把这条绑定标红并显示原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRegistrationFailure {
    /// 这条绑定在配置中的位置（从 0 开始）。
    pub index: usize,
    pub binding: HotkeyBinding,
    pub error: RegistrationError,
}

/// 热键绑定注册失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// 与配置中位置更靠前的那条绑定（下标从 0 开始）使用了同一个热键。
    DuplicateOf(usize),
    /// 系统拒绝注册，例如热键已被其他程序占用。
    Platform(PlatformError),
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationError::DuplicateOf(index) => {
                write!(f, "与第 {} 条热键绑定的热键重复", index + 1)
            }
            RegistrationError::Platform(e) => write!(f, "{e}"),
        }
    }
}
