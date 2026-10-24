//! 全局热键：`RegisterHotKey` 绑定到事件循环的隐藏窗口，按下时收到 `WM_HOTKEY`。

use bps_platform::{GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers, PlatformError};
use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey, VIRTUAL_KEY, VK_F1, VK_SNAPSHOT,
};

use crate::error::win_error;

/// 绑定到某个窗口的全局热键集合；被丢弃时注销仍处于注册状态的热键。
pub struct WindowsHotkeys {
    hwnd: HWND,
    registered: Vec<HotkeyId>,
}

impl WindowsHotkeys {
    pub(crate) fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered: Vec::new(),
        }
    }
}

impl GlobalHotkeys for WindowsHotkeys {
    fn register(&mut self, id: HotkeyId, hotkey: Hotkey) -> Result<(), PlatformError> {
        let vk = virtual_key(hotkey.key)?;
        let modifiers = modifier_flags(hotkey.modifiers) | MOD_NOREPEAT;
        unsafe { RegisterHotKey(Some(self.hwnd), native_id(id)?, modifiers, u32::from(vk.0)) }
            .map_err(|e| {
                if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() {
                    PlatformError::new(format!("热键 {hotkey:?} 已被其他程序占用"))
                } else {
                    win_error(&format!("注册热键 {hotkey:?} 失败"), e)
                }
            })?;
        self.registered.push(id);
        Ok(())
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.registered.retain(|r| *r != id);
        unsafe { UnregisterHotKey(Some(self.hwnd), native_id(id)?) }
            .map_err(|e| win_error("注销热键失败", e))
    }
}

impl Drop for WindowsHotkeys {
    fn drop(&mut self) {
        for id in std::mem::take(&mut self.registered) {
            let _ = self.unregister(id);
        }
    }
}

/// 应用程序可用的热键标识范围是 0x0000–0xBFFF。
fn native_id(id: HotkeyId) -> Result<i32, PlatformError> {
    if id.0 <= 0xBFFF {
        Ok(id.0 as i32)
    } else {
        Err(PlatformError::new(format!("热键标识 {id:?} 超出范围")))
    }
}

fn modifier_flags(modifiers: Modifiers) -> HOT_KEY_MODIFIERS {
    let mut flags = HOT_KEY_MODIFIERS(0);
    for (on, flag) in [
        (modifiers.ctrl, MOD_CONTROL),
        (modifiers.alt, MOD_ALT),
        (modifiers.shift, MOD_SHIFT),
        (modifiers.win, MOD_WIN),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

fn virtual_key(key: Key) -> Result<VIRTUAL_KEY, PlatformError> {
    match key {
        Key::PrintScreen => Ok(VK_SNAPSHOT),
        Key::F(n @ 1..=24) => Ok(VIRTUAL_KEY(VK_F1.0 + u16::from(n) - 1)),
        Key::F(n) => Err(PlatformError::new(format!("不存在功能键 F{n}"))),
    }
}
