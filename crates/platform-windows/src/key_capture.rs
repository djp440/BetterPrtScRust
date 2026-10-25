//! 录制热键用的低层键盘钩子（`WH_KEYBOARD_LL`）。
//!
//! egui / winit 拿不到 PrintScreen 和 Win 组合键，系统也会先处理它们，所以设置界面录制热键时
//! 装一个低层钩子：期间所有按键都交给录制、不再传给系统和其他程序（Win 键不会打开开始菜单，
//! 常驻实例的全局热键也不会被触发）。钩子只在 [`start_key_capture`] 到 [`KeyCapture`] 被丢弃之间
//! 存在，丢弃时立即卸载；接收端不在了（界面线程卡死或已退出）时钩子自己放行所有按键。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use bps_platform::{Key, KeyInput, ModifierKey, PlatformError, RecordedKey};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_UP, MSG,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_QUIT,
};

use crate::error::win_error;

/// 钩子回调是裸函数，拿不到上下文：把发送端放在进程级变量里（同一时刻只允许一个录制）。
static SENDER: Mutex<Option<Sender<KeyInput>>> = Mutex::new(None);

fn sender() -> MutexGuard<'static, Option<Sender<KeyInput>>> {
    SENDER.lock().unwrap_or_else(|e| e.into_inner())
}

/// 正在进行的按键录制；丢弃时卸载钩子。
pub struct KeyCapture {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for KeyCapture {
    fn drop(&mut self) {
        // 先让钩子放行，再让钩子线程退出消息循环并卸载。
        *sender() = None;
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 开始录制：装上低层键盘钩子。返回按键事件（按下和松开）的接收端，和丢弃即卸载钩子的守卫。
/// 已有一个录制在进行时失败。
pub fn start_key_capture() -> Result<(Receiver<KeyInput>, KeyCapture), PlatformError> {
    let (input_sender, inputs) = mpsc::channel();
    {
        let mut slot = sender();
        if slot.is_some() {
            return Err(PlatformError::new("已经有一个按键录制在进行"));
        }
        *slot = Some(input_sender);
    }
    let (ready_sender, ready) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("bps-key-capture".to_owned())
        .spawn(move || hook_thread(ready_sender));
    let fail = |error: PlatformError| {
        *sender() = None;
        error
    };
    let thread = spawned.map_err(|e| fail(PlatformError::new(format!("无法启动录制线程：{e}"))))?;
    match ready.recv() {
        Ok(Ok(thread_id)) => Ok((
            inputs,
            KeyCapture {
                thread_id,
                thread: Some(thread),
            },
        )),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(fail(error))
        }
        Err(_) => Err(fail(PlatformError::new("录制线程意外退出"))),
    }
}

fn hook_thread(ready: Sender<Result<u32, PlatformError>>) {
    let hook = unsafe {
        GetModuleHandleW(None)
            .map_err(|e| win_error("读取模块句柄失败", e))
            .and_then(|module| {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0)
                    .map_err(|e| win_error("安装键盘钩子失败", e))
            })
    };
    let hook = match hook {
        Ok(hook) => hook,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    // 低层钩子靠安装它的线程的消息循环驱动。
    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let input = KeyInput {
            key: recorded_key(info.vkCode),
            pressed: info.flags.0 & LLKHF_UP.0 == 0,
        };
        if let Some(sender) = sender().as_ref()
            && sender.send(input).is_ok()
        {
            // 吞掉：不再传给系统和其他程序。
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 虚拟键码 → 录制用的按键分类。
pub fn recorded_key(vk: u32) -> RecordedKey {
    let modifier = |modifier, right| RecordedKey::Modifier { modifier, right };
    match vk {
        0x10 | 0xA0 => modifier(ModifierKey::Shift, false),
        0xA1 => modifier(ModifierKey::Shift, true),
        0x11 | 0xA2 => modifier(ModifierKey::Ctrl, false),
        0xA3 => modifier(ModifierKey::Ctrl, true),
        0x12 | 0xA4 => modifier(ModifierKey::Alt, false),
        0xA5 => modifier(ModifierKey::Alt, true),
        0x5B => modifier(ModifierKey::Win, false),
        0x5C => modifier(ModifierKey::Win, true),
        0x1B => RecordedKey::Escape,
        0x08 => RecordedKey::Backspace,
        0x2E => RecordedKey::Delete,
        0x2C => RecordedKey::Main(Key::PrintScreen),
        0x70..=0x87 => RecordedKey::Main(Key::F((vk - 0x70 + 1) as u8)),
        0x30..=0x39 => RecordedKey::Main(Key::Digit((vk - 0x30) as u8)),
        0x41..=0x5A => RecordedKey::Main(Key::Letter(vk as u8 as char)),
        _ => RecordedKey::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keys_map_to_recordable_keys() {
        assert_eq!(recorded_key(0x2C), RecordedKey::Main(Key::PrintScreen));
        assert_eq!(recorded_key(0x7A), RecordedKey::Main(Key::F(11)));
        assert_eq!(recorded_key(0x87), RecordedKey::Main(Key::F(24)));
        assert_eq!(recorded_key(0x4F), RecordedKey::Main(Key::Letter('O')));
        assert_eq!(recorded_key(0x35), RecordedKey::Main(Key::Digit(5)));
        assert_eq!(recorded_key(0x1B), RecordedKey::Escape);
        assert_eq!(recorded_key(0x20), RecordedKey::Other);
        assert_eq!(
            recorded_key(0x5C),
            RecordedKey::Modifier {
                modifier: ModifierKey::Win,
                right: true
            }
        );
        assert_eq!(
            recorded_key(0xA0),
            RecordedKey::Modifier {
                modifier: ModifierKey::Shift,
                right: false
            }
        );
    }
}
