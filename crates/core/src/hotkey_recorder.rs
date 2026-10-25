//! 设置界面的“录制热键”：把一串按键事件解析成 [`Hotkey`]（与配置文件里的热键文本一致）。
//!
//! 事件来自平台的低层键盘钩子（拿得到 PrintScreen 和 Win 组合）；这里只管状态：
//! 修饰键按下时累积，第一个可作主键的键按下就记录；Esc 取消；Backspace / Delete 清空。

use bps_platform::{Hotkey, Key, KeyInput, ModifierKey, Modifiers, RecordedKey};

/// 喂入一个按键后的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingOutcome {
    /// 还在等主键；附带当前按住的修饰键，用来实时预览。
    Waiting(Modifiers),
    /// 录到了热键。
    Recorded(Hotkey),
    /// 用户按了 Esc：取消录制，保持原热键。
    Cancelled,
    /// 用户按了 Backspace / Delete：清空热键。
    Cleared,
    /// 按了热键不支持的键，继续等待。
    Unsupported,
    /// 已经有了结论，之后的按键只用来跟踪何时全部松开。
    Finished,
}

/// 录制状态。
#[derive(Debug, Clone, Default)]
pub struct HotkeyRecorder {
    /// 按住的修饰键：低 4 位是左侧的 Ctrl/Alt/Shift/Win，高 4 位是右侧。
    held_modifiers: u8,
    /// 按住的主键数（含 Esc 等）。
    held_other: u32,
    finished: bool,
}

impl HotkeyRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前按住的修饰键。
    pub fn modifiers(&self) -> Modifiers {
        let held = |m: ModifierKey| self.held_modifiers & (bit(m, false) | bit(m, true)) != 0;
        Modifiers {
            ctrl: held(ModifierKey::Ctrl),
            alt: held(ModifierKey::Alt),
            shift: held(ModifierKey::Shift),
            win: held(ModifierKey::Win),
        }
    }

    /// 所有键都已松开（结论出来后，钩子要等到这时才卸载，免得残留的松开事件泄漏给系统）。
    pub fn all_released(&self) -> bool {
        self.held_modifiers == 0 && self.held_other == 0
    }

    /// 处理一个按键事件。
    pub fn feed(&mut self, input: KeyInput) -> RecordingOutcome {
        // PrintScreen 有时只递交松开：没有先按下的松开也当作一次按下。
        let print_screen_release_only = !input.pressed
            && input.key == RecordedKey::Main(Key::PrintScreen)
            && !self.finished
            && self.held_other == 0;
        self.track(input);
        if self.finished {
            return RecordingOutcome::Finished;
        }
        if !input.pressed && !print_screen_release_only {
            return RecordingOutcome::Waiting(self.modifiers());
        }
        let outcome = match input.key {
            RecordedKey::Modifier { .. } => RecordingOutcome::Waiting(self.modifiers()),
            RecordedKey::Escape => RecordingOutcome::Cancelled,
            RecordedKey::Backspace | RecordedKey::Delete if self.modifiers() == Modifiers::NONE => {
                RecordingOutcome::Cleared
            }
            RecordedKey::Backspace | RecordedKey::Delete | RecordedKey::Other => {
                RecordingOutcome::Unsupported
            }
            RecordedKey::Main(key) => {
                RecordingOutcome::Recorded(Hotkey::new(self.modifiers(), key))
            }
        };
        if !matches!(
            outcome,
            RecordingOutcome::Waiting(_) | RecordingOutcome::Unsupported
        ) {
            self.finished = true;
        }
        outcome
    }

    fn track(&mut self, input: KeyInput) {
        match input.key {
            RecordedKey::Modifier { modifier, right } => {
                let mask = bit(modifier, right);
                if input.pressed {
                    self.held_modifiers |= mask;
                } else {
                    self.held_modifiers &= !mask;
                }
            }
            _ if input.pressed => {
                // 自动重复会反复按下同一个键，按下次数不能无限累加。
                self.held_other = 1.max(self.held_other);
            }
            _ => self.held_other = 0,
        }
    }
}

fn bit(modifier: ModifierKey, right: bool) -> u8 {
    let index = match modifier {
        ModifierKey::Ctrl => 0,
        ModifierKey::Alt => 1,
        ModifierKey::Shift => 2,
        ModifierKey::Win => 3,
    };
    1 << (index + if right { 4 } else { 0 })
}
