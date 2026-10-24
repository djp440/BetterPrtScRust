//! 通知：托盘图标的气泡（Win10 起显示为系统通知），点击时打开文件或文件夹。

use std::path::Path;
use std::rc::Rc;

use bps_platform::{Notification, NotificationClick, Notifier};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::UI::Shell::{
    NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::wide_path;
use crate::event_loop::{Shared, TRAY_ICON_ID};

/// 通过事件循环的托盘图标发出通知；托盘图标不存在时通知会丢失（只打印到控制台）。
pub struct WindowsNotifier {
    hwnd: HWND,
    shared: Rc<Shared>,
}

impl WindowsNotifier {
    pub(crate) fn new(hwnd: HWND, shared: Rc<Shared>) -> Self {
        Self { hwnd, shared }
    }
}

impl Notifier for WindowsNotifier {
    fn notify(&self, notification: Notification) {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_truncated(&mut data.szInfoTitle, &notification.title);
        // 正文为空表示“收起气泡”，所以至少放一个空格。
        let body = if notification.body.is_empty() {
            " "
        } else {
            &notification.body
        };
        copy_truncated(&mut data.szInfo, body);
        *self.shared.balloon_click.borrow_mut() = notification.on_click;
        // 控制台（debug 版）同时记一份，便于脚本和开发者观察。
        let delivered = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool();
        eprintln!(
            "[通知{}] {}：{}",
            if delivered { "" } else { "发送失败" },
            notification.title,
            notification.body.replace('\n', " / ")
        );
    }
}

/// 把 `text` 写成以 NUL 结尾的 UTF-16，放不下时截断并以“…”结尾。
pub(crate) fn copy_truncated(target: &mut [u16], text: &str) {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    let capacity = target.len() - 1;
    if units.len() > capacity {
        units.truncate(capacity - 1);
        // 不要留下半个代理对。
        if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            units.pop();
        }
        units.push('…' as u16);
    }
    target[..units.len()].copy_from_slice(&units);
    target[units.len()] = 0;
}

/// 用户点击了通知：执行它附带的操作。
pub(crate) fn open(hwnd: HWND, click: NotificationClick) {
    // ShellExecute 可能交给 Shell 扩展处理，要求线程已初始化 COM（重复初始化无害）。
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    match click {
        NotificationClick::OpenFile(path) => {
            // 没有关联程序（例如 .toml）时退而在资源管理器中选中它。
            if !shell_open(hwnd, &path) {
                let select = HSTRING::from(format!("/select,\"{}\"", path.display()));
                let _ = shell_execute(hwnd, w!("explorer.exe"), &select);
            }
        }
        NotificationClick::OpenFolder(path) => {
            let _ = shell_open(hwnd, &path);
        }
    }
}

fn shell_open(hwnd: HWND, path: &Path) -> bool {
    shell_execute(hwnd, &wide_path(path), &HSTRING::new())
}

fn shell_execute(
    hwnd: HWND,
    file: impl windows::core::Param<PCWSTR>,
    parameters: &HSTRING,
) -> bool {
    let result = unsafe {
        ShellExecuteW(
            Some(hwnd),
            w!("open"),
            file,
            parameters,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 大于 32 表示成功。
    result.0 as isize > 32
}
