//! 录制热键用的低层键盘钩子（`WH_KEYBOARD_LL`）。
//!
//! egui / winit 拿不到 PrintScreen 和 Win 组合键，系统也会先处理它们，所以设置界面录制热键时
//! 装一个低层钩子：期间所有按键都交给录制、不再传给系统和其他程序（Win 键不会打开开始菜单，
//! 常驻实例的全局热键也不会被触发）。钩子只在 [`start_key_capture`] 到 [`KeyCapture`] 被丢弃之间
//! 存在，丢弃时立即卸载；接收端不在了（界面线程卡死或已退出）时钩子自己放行所有按键。
//!
//! 实测在已经创建了 eframe（winit + OpenGL）窗口的进程里，同一个钩子收不到任何按键，
//! 在没有窗口的进程里一切正常。所以设置界面不在自己的进程里装钩子，而是按需启动一个
//! 没有窗口的录制子进程（`BetterPrtSc.exe --record-keys`，[`spawn_key_capture`]），
//! 子进程把按键按行写到标准输出；界面进程退出（标准输入关闭）、写入失败或超时，子进程立即结束，
//! 钩子随进程一起卸载。

use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
static SENDER: Mutex<Option<Sender<RawKey>>> = Mutex::new(None);

/// 录制子进程的命令行参数。
pub const RECORD_KEYS_ARGUMENT: &str = "--record-keys";

/// 录制子进程最长存活时间：界面进程卡死也不会让钩子一直吞按键。
const CHILD_MAX_LIFETIME: Duration = Duration::from_secs(60);
/// 等录制子进程装好钩子的最长时间。
const CHILD_START_TIMEOUT: Duration = Duration::from_secs(5);

/// 钩子收到的一次按下或松开（虚拟键码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub vk: u32,
    pub pressed: bool,
}

impl RawKey {
    /// 分类成录制用的按键事件。
    pub fn input(self) -> KeyInput {
        KeyInput {
            key: recorded_key(self.vk),
            pressed: self.pressed,
        }
    }

    fn encode(self) -> String {
        format!("{} {}", if self.pressed { 'd' } else { 'u' }, self.vk)
    }

    fn decode(line: &str) -> Option<RawKey> {
        let (kind, vk) = line.split_once(' ')?;
        let pressed = match kind {
            "d" => true,
            "u" => false,
            _ => return None,
        };
        Some(RawKey {
            vk: vk.trim().parse().ok()?,
            pressed,
        })
    }
}

fn sender() -> MutexGuard<'static, Option<Sender<RawKey>>> {
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
pub fn start_key_capture() -> Result<(Receiver<RawKey>, KeyCapture), PlatformError> {
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
        let input = RawKey {
            vk: info.vkCode,
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

/// 录制子进程的入口（`--record-keys`）：装上钩子，把按键按行写到标准输出，
/// 先写一行 `ready`（装失败写 `error <说明>`）。标准输入关闭、写入失败或超时就结束。
pub fn run_key_capture_child() -> Result<(), PlatformError> {
    let mut out = std::io::stdout();
    let (keys, _capture) = match start_key_capture() {
        Ok(started) => started,
        Err(error) => {
            let _ = writeln!(
                out,
                "error {}",
                error.to_string().replace(['\r', '\n'], " ")
            );
            let _ = out.flush();
            return Err(error);
        }
    };
    // 界面进程退出（含崩溃）时管道关闭，读到 EOF 就退出，钩子随进程卸载。
    thread::spawn(|| {
        let mut sink = [0u8; 64];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        std::process::exit(0);
    });
    if writeln!(out, "ready").and_then(|()| out.flush()).is_err() {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + CHILD_MAX_LIFETIME;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match keys.recv_timeout(left) {
            Ok(key) => {
                if writeln!(out, "{}", key.encode())
                    .and_then(|()| out.flush())
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}

/// 界面进程一侧的录制子进程；丢弃时结束子进程（钩子随之卸载）。
pub struct KeyCaptureProcess {
    child: Child,
}

impl Drop for KeyCaptureProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 启动录制子进程（`exe` 是 BetterPrtSc.exe），等它装好钩子后返回按键事件的接收端。
pub fn spawn_key_capture(
    exe: &Path,
) -> Result<(Receiver<KeyInput>, KeyCaptureProcess), PlatformError> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new(exe)
        .arg(RECORD_KEYS_ARGUMENT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| PlatformError::new(format!("无法启动按键录制进程：{e}")))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(PlatformError::new("无法读取按键录制进程的输出"));
    };
    let process = KeyCaptureProcess { child };
    let (ready_sender, ready) = mpsc::channel();
    let (input_sender, inputs) = mpsc::channel();
    thread::Builder::new()
        .name("bps-key-capture-reader".to_owned())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line == "ready" {
                    let _ = ready_sender.send(Ok(()));
                } else if let Some(reason) = line.strip_prefix("error ") {
                    let _ = ready_sender.send(Err(reason.to_owned()));
                } else if let Some(key) = RawKey::decode(&line)
                    && input_sender.send(key.input()).is_err()
                {
                    break;
                }
            }
            let _ = ready_sender.send(Err("按键录制进程意外退出".to_owned()));
        })
        .map_err(|e| PlatformError::new(format!("无法启动读取线程：{e}")))?;
    match ready.recv_timeout(CHILD_START_TIMEOUT) {
        Ok(Ok(())) => Ok((inputs, process)),
        Ok(Err(reason)) => Err(PlatformError::new(reason)),
        Err(_) => Err(PlatformError::new("按键录制进程没有及时就绪")),
    }
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
    fn raw_keys_round_trip_through_the_child_protocol() {
        for key in [
            RawKey {
                vk: 0xA2,
                pressed: true,
            },
            RawKey {
                vk: 0x2C,
                pressed: false,
            },
        ] {
            assert_eq!(RawKey::decode(&key.encode()), Some(key));
        }
        assert_eq!(
            RawKey::decode("d 44").unwrap().input(),
            KeyInput::down(RecordedKey::Main(Key::PrintScreen))
        );
        assert_eq!(RawKey::decode("ready"), None);
        assert_eq!(RawKey::decode("x 4"), None);
    }

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
