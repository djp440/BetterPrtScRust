use std::fmt;
use std::str::FromStr;

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
    /// 字母键 A–Z，用大写字母表示。
    Letter(char),
    /// 数字键 0–9（主键盘区）。
    Digit(u8),
}

impl Key {
    /// 字母键和数字键：单独或只配 Shift 注册成全局热键会吞掉正常输入。
    pub fn is_typing_key(self) -> bool {
        matches!(self, Key::Letter(_) | Key::Digit(_))
    }
}

/// 一个全局热键：修饰键 + 主键。
///
/// 文本形式是用 `+` 连接的修饰键和主键，例如 `Ctrl+Shift+PrintScreen`、`Win+Shift+O`：
/// 解析时不区分大小写，`Control` 可代替 `Ctrl`，`PrtSc` 可代替 `PrintScreen`；
/// 输出时按 `Win`、`Ctrl`、`Alt`、`Shift` 的顺序排列修饰键。
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

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::PrintScreen => f.write_str("PrintScreen"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.win, "Win+"),
            (m.ctrl, "Ctrl+"),
            (m.alt, "Alt+"),
            (m.shift, "Shift+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// 热键文本无法解析。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseHotkeyError(String);

impl fmt::Display for ParseHotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseHotkeyError {}

impl FromStr for Hotkey {
    type Err = ParseHotkeyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = |message: String| Err(ParseHotkeyError(message));
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key_name = parts.pop().unwrap_or_default();
        let mut modifiers = Modifiers::NONE;
        for part in parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "win" => &mut modifiers.win,
                _ => {
                    return error(format!(
                        "热键“{text}”中的“{part}”不是修饰键（Ctrl、Alt、Shift、Win）"
                    ));
                }
            };
            *flag = true;
        }
        match parse_key(key_name) {
            Some(key) => Ok(Hotkey::new(modifiers, key)),
            None if key_name.is_empty() => error(format!("热键“{text}”缺少主键")),
            None => error(format!("热键“{text}”中的“{key_name}”不是支持的按键")),
        }
    }
}

fn parse_key(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "printscreen" | "prtsc" => return Some(Key::PrintScreen),
        _ => {}
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match c {
            'a'..='z' | 'A'..='Z' => Some(Key::Letter(c.to_ascii_uppercase())),
            '0'..='9' => Some(Key::Digit(c as u8 - b'0')),
            _ => None,
        };
    }
    match lower.strip_prefix('f')?.parse::<u8>() {
        Ok(n @ 1..=24) => Some(Key::F(n)),
        _ => None,
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
