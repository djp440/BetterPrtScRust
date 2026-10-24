use crate::PlatformError;

/// 修饰键组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };
    pub const CTRL: Modifiers = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    pub const ALT: Modifiers = Modifiers {
        alt: true,
        ..Modifiers::NONE
    };
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..Modifiers::NONE
    };
    pub const WIN: Modifiers = Modifiers {
        win: true,
        ..Modifiers::NONE
    };

    /// 两组修饰键的并集，例如 `Modifiers::CTRL.with(Modifiers::SHIFT)`。
    pub const fn with(self, other: Modifiers) -> Modifiers {
        Modifiers {
            ctrl: self.ctrl || other.ctrl,
            alt: self.alt || other.alt,
            shift: self.shift || other.shift,
            win: self.win || other.win,
        }
    }
}

/// 热键的主键，按需扩充。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    PrintScreen,
    /// 功能键 F1–F24。
    F(u8),
}

/// 一个全局热键：修饰键 + 主键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Hotkey {
    pub const fn new(modifiers: Modifiers, key: Key) -> Self {
        Self { modifiers, key }
    }
}

/// 常驻实例为每个已注册热键分配的标识，按下时随 [`crate::PlatformEvent`] 返回。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyId(pub u32);

/// 全局热键能力。
pub trait GlobalHotkeys {
    /// 注册全局热键；热键已被其他程序占用等情况返回错误。
    fn register(&mut self, id: HotkeyId, hotkey: Hotkey) -> Result<(), PlatformError>;

    /// 注销之前用 `id` 注册的热键。
    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError>;
}
