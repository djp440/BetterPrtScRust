use bps_platform::{Hotkey, Key, Modifiers, PlatformError};

use crate::{CaptureMode, CaptureRequest, PostCaptureAction};

/// 热键绑定：一个全局热键对应一组“捕获模式 + 截后动作”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub hotkey: Hotkey,
    pub capture: CaptureRequest,
}

/// 内置的默认热键绑定。目前只有 Shift+PrtSc → 显示器 + 复制到剪贴板 + 自动存储。
pub fn default_bindings() -> Vec<HotkeyBinding> {
    vec![HotkeyBinding {
        hotkey: Hotkey::new(Modifiers::SHIFT, Key::PrintScreen),
        capture: CaptureRequest::new(
            CaptureMode::Display,
            vec![
                PostCaptureAction::CopyToClipboard,
                PostCaptureAction::AutoSave,
            ],
        ),
    }]
}

/// 一条热键绑定注册失败（例如热键已被其他程序占用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRegistrationFailure {
    pub binding: HotkeyBinding,
    pub error: PlatformError,
}
