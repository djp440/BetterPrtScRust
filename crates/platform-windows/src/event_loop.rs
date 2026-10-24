//! 常驻实例的宿主：隐藏窗口 + 消息循环 + 托盘图标。
//!
//! 热键（`WM_HOTKEY`）、托盘菜单、关闭请求都在窗口过程里转换成 [`PlatformEvent`]
//! 放进队列，由 [`EventLoop::run`] 在窗口过程之外逐个交给处理函数，
//! 因此处理函数里执行耗时的捕获不会重入窗口过程。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use bps_platform::{HotkeyId, Image, PlatformError, PlatformEvent};
use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, HICON, ICONINFO, MF_STRING, MSG, MsgWaitForMultipleObjects, PM_REMOVE,
    PeekMessageW, PostMessageW, QS_ALLINPUT, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, SetWindowLongPtrW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_HOTKEY, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::WindowsHotkeys;
use crate::error::win_error;

/// 隐藏窗口的类名；其他进程可以据此找到常驻实例。
const CLASS_NAME: PCWSTR = w!("BetterPrtSc.ResidentWindow");
/// 托盘图标回调消息。
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ICON_ID: u32 = 1;
const MENU_EXIT: usize = 1;

/// 常驻实例的消息循环。一个线程只应创建一个。
pub struct EventLoop {
    hwnd: HWND,
    /// 窗口过程通过 `GWLP_USERDATA` 访问它，所以放在堆上保证地址不变。
    state: Box<WindowState>,
}

struct WindowState {
    events: RefCell<VecDeque<PlatformEvent>>,
    tray: RefCell<Option<TrayIcon>>,
    /// 资源管理器重启后广播的消息，收到时重新添加托盘图标。
    taskbar_created: u32,
}

impl WindowState {
    fn push(&self, event: PlatformEvent) {
        self.events.borrow_mut().push_back(event);
    }

    fn pop(&self) -> Option<PlatformEvent> {
        self.events.borrow_mut().pop_front()
    }
}

impl EventLoop {
    /// 创建隐藏窗口（不显示、不出现在任务栏）。
    pub fn new() -> Result<Self, PlatformError> {
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| win_error("读取模块句柄失败", e))?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: module.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(PlatformError::new("注册窗口类失败"));
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("BetterPrtSc"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module.into()),
                None,
            )
            .map_err(|e| win_error("创建隐藏窗口失败", e))?;
            let state = Box::new(WindowState {
                events: RefCell::new(VecDeque::new()),
                tray: RefCell::new(None),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const WindowState as isize);
            Ok(Self { hwnd, state })
        }
    }

    /// 显示托盘图标；右键或左键单击弹出只有“退出”一项的菜单。
    pub fn show_tray_icon(&self, tooltip: &str, icon: &Image) -> Result<(), PlatformError> {
        let tray = TrayIcon {
            icon: create_icon(icon)?,
            tooltip: tooltip.to_owned(),
        };
        tray.add(self.hwnd)?;
        if let Some(old) = self.state.tray.borrow_mut().replace(tray) {
            old.icon_destroy();
        }
        Ok(())
    }

    /// 绑定到本窗口的全局热键；按下时本循环递交 [`PlatformEvent::HotkeyPressed`]。
    pub fn hotkeys(&self) -> WindowsHotkeys {
        WindowsHotkeys::new(self.hwnd)
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 运行消息循环，把每个事件交给 `handler`，直到它返回 `Break`。
    pub fn run(&self, mut handler: impl FnMut(PlatformEvent) -> ControlFlow<()>) {
        let mut msg = MSG::default();
        loop {
            while let Some(event) = self.state.pop() {
                if handler(event).is_break() {
                    return;
                }
            }
            let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if result.0 == 0 || result.0 == -1 {
                return;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// 处理消息直到出现一个事件或超时，供冒烟测试使用。
    pub fn wait_event(&self, timeout: Duration) -> Option<PlatformEvent> {
        let deadline = Instant::now() + timeout;
        let mut msg = MSG::default();
        loop {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if let Some(event) = self.state.pop() {
                return Some(event);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            unsafe {
                MsgWaitForMultipleObjects(None, false, remaining.as_millis() as u32, QS_ALLINPUT);
            }
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        if let Some(tray) = self.state.tray.borrow_mut().take() {
            tray.remove(self.hwnd);
            tray.icon_destroy();
        }
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
    if let Some(state) = unsafe { state.as_ref() } {
        match msg {
            WM_HOTKEY => {
                state.push(PlatformEvent::HotkeyPressed(HotkeyId(wparam.0 as u32)));
                return LRESULT(0);
            }
            WM_TRAY => {
                let mouse = (lparam.0 & 0xFFFF) as u32;
                if (mouse == WM_RBUTTONUP || mouse == WM_LBUTTONUP)
                    && show_tray_menu(hwnd) == Some(MENU_EXIT)
                {
                    state.push(PlatformEvent::ExitRequested);
                }
                return LRESULT(0);
            }
            WM_CLOSE => {
                state.push(PlatformEvent::ExitRequested);
                return LRESULT(0);
            }
            m if m != 0 && m == state.taskbar_created => {
                if let Some(tray) = state.tray.borrow().as_ref() {
                    let _ = tray.add(hwnd);
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 在鼠标位置弹出托盘菜单，返回选中的菜单项。
fn show_tray_menu(hwnd: HWND) -> Option<usize> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("退出"));
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // 不先置前，点击菜单外部时菜单不会关闭（KB135788）。
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        (command.0 > 0).then_some(command.0 as usize)
    }
}

struct TrayIcon {
    icon: HICON,
    tooltip: String,
}

impl TrayIcon {
    fn data(&self, hwnd: HWND) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip: Vec<u16> = self.tooltip.encode_utf16().collect();
        let len = tip.len().min(data.szTip.len() - 1);
        data.szTip[..len].copy_from_slice(&tip[..len]);
        data
    }

    fn add(&self, hwnd: HWND) -> Result<(), PlatformError> {
        let data = self.data(hwnd);
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            Ok(())
        } else {
            Err(PlatformError::new("添加托盘图标失败"))
        }
    }

    fn remove(&self, hwnd: HWND) {
        let data = self.data(hwnd);
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }

    fn icon_destroy(&self) {
        let _ = unsafe { DestroyIcon(self.icon) };
    }
}

/// 用 RGBA 图像创建图标（32 位带 alpha）。
fn create_icon(image: &Image) -> Result<HICON, PlatformError> {
    let (width, height) = (image.width() as i32, image.height() as i32);
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // 负高度：自上而下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| win_error("创建图标位图失败", e))?;
        let target = std::slice::from_raw_parts_mut(bits as *mut u8, image.rgba().len());
        for (dst, src) in target.chunks_exact_mut(4).zip(image.rgba().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        // 32 位图标使用 alpha 通道，掩码位图全 0 即可（单色位图每行按 2 字节对齐）。
        let mask_bytes = vec![0u8; (width as usize).div_ceil(16) * 2 * height as usize];
        let mask = CreateBitmap(width, height, 1, 1, Some(mask_bytes.as_ptr().cast()));
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.map_err(|e| win_error("创建图标失败", e))
    }
}
