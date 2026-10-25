use crate::Key;

/// 录制热键时收到的一个按键（与具体键盘布局、左右无关的分类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedKey {
    /// 修饰键；`right` 表示右侧那一个（左右两个键分开计数，松开才不会互相干扰）。
    Modifier {
        modifier: ModifierKey,
        right: bool,
    },
    Escape,
    Backspace,
    Delete,
    /// 可以作为热键主键的键。
    Main(Key),
    /// 其他按键（热键不支持）。
    Other,
}

/// 四种修饰键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Ctrl,
    Alt,
    Shift,
    Win,
}

/// 一次按下或松开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    pub key: RecordedKey,
    pub pressed: bool,
}

impl KeyInput {
    pub const fn down(key: RecordedKey) -> Self {
        Self { key, pressed: true }
    }

    pub const fn up(key: RecordedKey) -> Self {
        Self {
            key,
            pressed: false,
        }
    }
}
