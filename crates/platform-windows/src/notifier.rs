//! 通知：托盘图标的气泡（Win10/11 上显示为系统通知）。
//!
//! 点击气泡时外壳向托盘回调发送 `NIN_BALLOONUSERCLICK`，事件循环据此执行
//! 最近一条通知的点击操作（见 [`open_click_target`]）。

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::rc::Rc;

use bps_platform::{Notification, NotificationClick, Notifier};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::event_loop::TRAY_ICON_ID;

/// 最近一条通知的点击操作，由通知器写入、事件循环在用户点击气泡时取出。
pub(crate) type PendingClick = Rc<RefCell<Option<NotificationClick>>>;

/// 用常驻实例的托盘图标显示气泡通知。托盘图标不存在时通知不会显示。
pub struct TrayNotifier {
    hwnd: HWND,
    pending_click: PendingClick,
}

impl TrayNotifier {
    pub(crate) fn new(hwnd: HWND, pending_click: PendingClick) -> Self {
        Self {
            hwnd,
            pending_click,
        }
    }
}

impl Notifier for TrayNotifier {
    fn notify(&self, notification: Notification) {
        // 同时写一行日志，便于排查和冒烟脚本确认发出了哪条通知。
        eprintln!("[通知] {}：{}", notification.title, notification.body);
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        fill_wide(&mut data.szInfoTitle, &notification.title);
        fill_wide(&mut data.szInfo, &notification.body);
        *self.pending_click.borrow_mut() = notification.on_click;
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            eprintln!("显示托盘通知失败：{}", notification.title);
        }
    }
}

/// “设置”应用里含“使用 Print Screen 键打开屏幕截图”开关的“键盘”页（Win10 / Win11 相同）。
const PRINT_SCREEN_KEY_SETTINGS_URI: &str = "ms-settings:easeofaccess-keyboard";

/// 执行通知的点击操作：用默认程序打开文件或网址，在资源管理器中打开文件夹，或打开系统设置页面。
pub(crate) fn open_click_target(hwnd: HWND, click: &NotificationClick) {
    let (verb, target) = match click {
        NotificationClick::OpenFile(path) => (w!("open"), path.as_os_str()),
        NotificationClick::OpenFolder(path) => (w!("explore"), path.as_os_str()),
        NotificationClick::OpenPrintScreenKeySettings => {
            (w!("open"), OsStr::new(PRINT_SCREEN_KEY_SETTINGS_URI))
        }
        // 只是让浏览器打开发布页面，本程序不下载任何东西。
        NotificationClick::OpenUrl(url) => (w!("open"), OsStr::new(url.as_str())),
    };
    let wide: Vec<u16> = target.encode_wide().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            Some(hwnd),
            verb,
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 返回值大于 32 表示成功。
    if result.0 as usize <= 32 {
        eprintln!(
            "打开 {} 失败（ShellExecute 返回 {}）",
            target.to_string_lossy(),
            result.0 as usize
        );
    }
}

/// 把 `text` 写进以 0 结尾的定长 UTF-16 缓冲区，过长时截断。
fn fill_wide(buffer: &mut [u16], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let len = wide.len().min(buffer.len() - 1);
    buffer[..len].copy_from_slice(&wide[..len]);
    buffer[len] = 0;
}
