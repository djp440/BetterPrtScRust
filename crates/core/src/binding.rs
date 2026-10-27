use std::fmt;

use bps_platform::{Hotkey, PlatformError};

use crate::{CaptureRequest, Config, Locale, Texts};

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
    /// 热键已被其他程序占用。
    TakenByOtherProgram,
    /// 系统因其他原因拒绝注册。
    Platform(PlatformError),
    /// 系统设置把 PrtSc 键交给了系统自带的截图工具（只影响不带修饰键的 PrtSc）。
    /// 用户在系统设置里关闭后会自动重新注册。
    TakenBySystemScreenshotTool,
}

impl RegistrationError {
    /// 用 `texts` 的语言说明失败原因。系统给出的其他原因按原文显示。
    pub fn localized(&self, texts: &Texts) -> String {
        match self {
            RegistrationError::DuplicateOf(index) => {
                texts.format("registration-duplicate", &[("index", &(index + 1))])
            }
            RegistrationError::TakenByOtherProgram => texts.get("registration-taken"),
            RegistrationError::TakenBySystemScreenshotTool => {
                texts.get("registration-taken-by-system-screenshot-tool")
            }
            RegistrationError::Platform(e) => e.to_string(),
        }
    }
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized(Texts::of(Locale::ZhCn)))
    }
}
