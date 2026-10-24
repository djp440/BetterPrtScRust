//! 常驻实例的宿主：隐藏窗口 + 消息循环 + 托盘图标。
//!
//! 热键（`WM_HOTKEY`）、托盘菜单（内容由 `core` 经 [`WindowsTrayMenu`] 设置）、关闭请求、
//! 配置文件变更都转换成 [`PlatformEvent`]
//! 放进队列，由 [`EventLoop::run`] 在窗口过程之外逐个交给处理函数，
//! 因此处理函数里执行耗时的捕获不会重入窗口过程。
//! 其他线程（命名管道等）经 [`EventSender`] 投递事件，同样进入这个队列；
//! 同一线程上的选区遮罩窗口把用户输入直接放进这个队列（[`EventQueue`]）。
//!
//! 配置文件和 PrtSc 键系统设置的监视不另开线程：循环用 `MsgWaitForMultipleObjectsEx` 同时等待
//! 消息、目录变更通知句柄和注册表变更事件，有变化后用定时器合并短时间内的多次变化，再递交事件。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bps_platform::{
    HotkeyId, Image, PlatformError, PlatformEvent, TrayMenu, TrayMenuItem, TrayMenuItemId,
};
use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, WAIT_OBJECT_0,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::Storage::FileSystem::FindNextChangeNotification;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetWindowLongPtrW,
    HICON, ICONINFO, KillTimer, MF_STRING, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx,
    PM_REMOVE, PeekMessageW, PostMessageW, QS_ALLINPUT, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_QUIT,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::config_file::ChangeNotification;
use crate::error::win_error;
use crate::notifier::{PendingClick, open_click_target};
use crate::registry::RegistryWatch;
use crate::ticker::WaitableTimer;
use crate::{
    D2dSelectionOverlay, TrayNotifier, WindowsConfigFile, WindowsHotkeys, WindowsScreenshotTool,
    WindowsTicker,
};

/// 隐藏窗口的类名；其他进程可以据此找到常驻实例。
const CLASS_NAME: PCWSTR = w!("BetterPrtSc.ResidentWindow");
/// 托盘图标回调消息。
const WM_TRAY: u32 = WM_APP + 1;
/// 其他线程经 [`EventSender`] 投递了事件，唤醒消息循环去取。
const WM_SENT_EVENT: u32 = WM_APP + 2;
pub(crate) const TRAY_ICON_ID: u32 = 1;
/// 配置文件所在目录变化后，等这么久没有新的变化才递交事件。
const CONFIG_SETTLE_TIMER: usize = 1;
const CONFIG_SETTLE_DELAY_MS: u32 = 300;
/// PrtSc 键的系统设置变化后，等这么久再递交事件：给系统留出释放 PrtSc 热键的时间。
const KEYBOARD_SETTLE_TIMER: usize = 2;
const KEYBOARD_SETTLE_DELAY_MS: u32 = 1000;

/// 常驻实例的消息循环。一个线程只应创建一个。
pub struct EventLoop {
    hwnd: HWND,
    /// 窗口过程通过 `GWLP_USERDATA` 访问它，所以放在堆上保证地址不变。
    state: Box<WindowState>,
    sender: mpsc::Sender<PlatformEvent>,
}

struct WindowState {
    events: EventQueue,
    /// 其他线程投递的事件，收到 `WM_SENT_EVENT` 时移进 `events`。
    sent: mpsc::Receiver<PlatformEvent>,
    tray: RefCell<Option<TrayIcon>>,
    /// 托盘菜单的内容，由 `core` 设置。
    menu: MenuItems,
    /// 最近一条托盘通知的点击操作。
    balloon_click: PendingClick,
    /// 资源管理器重启后广播的消息，收到时重新添加托盘图标。
    taskbar_created: u32,
    shared: Rc<Shared>,
}

/// 托盘菜单的内容，由 [`WindowsTrayMenu`] 写入、窗口过程弹出菜单时读取。
type MenuItems = Rc<RefCell<Vec<TrayMenuItem>>>;

/// 托盘图标的右键菜单。菜单在用户单击托盘图标时按当前内容弹出。
pub struct WindowsTrayMenu {
    items: MenuItems,
}

impl TrayMenu for WindowsTrayMenu {
    fn set_items(&mut self, items: Vec<TrayMenuItem>) {
        *self.items.borrow_mut() = items;
    }
}

/// 事件循环与它创建的配置文件、系统截图工具对象共享的状态。
#[derive(Default)]
pub(crate) struct Shared {
    /// 配置文件所在目录的变更通知。
    pub config_watch: RefCell<Option<ChangeNotification>>,
    /// PrtSc 键系统设置所在注册表键的变更通知。
    pub keyboard_watch: RefCell<Option<RegistryWatch>>,
    /// 节拍（录屏）的定时器；到点后递交 `PlatformEvent::Tick`。
    pub ticker: RefCell<Option<WaitableTimer>>,
}

/// 从其他线程向 [`EventLoop`] 投递事件的句柄，可以克隆、跨线程传递。
#[derive(Clone)]
pub struct EventSender {
    /// 隐藏窗口的句柄；`HWND` 不是 `Send`，所以存成整数。
    hwnd: isize,
    events: mpsc::Sender<PlatformEvent>,
}

impl EventSender {
    /// 投递一个事件并唤醒消息循环。事件循环已经结束时把事件原样退回。
    pub fn send(&self, event: PlatformEvent) -> Result<(), PlatformEvent> {
        self.events.send(event).map_err(|e| e.0)?;
        let hwnd = HWND(self.hwnd as *mut _);
        // 窗口已销毁时投递失败：事件随事件循环一起丢弃，请求方会收到“没有回复”。
        let _ = unsafe { PostMessageW(Some(hwnd), WM_SENT_EVENT, WPARAM(0), LPARAM(0)) };
        Ok(())
    }
}

/// 待处理事件的队列，只在事件循环所在的线程上使用。
#[derive(Clone, Default)]
pub(crate) struct EventQueue(Rc<RefCell<VecDeque<PlatformEvent>>>);

impl EventQueue {
    pub(crate) fn push_back(&self, event: PlatformEvent) {
        self.0.borrow_mut().push_back(event);
    }

    fn pop_front(&self) -> Option<PlatformEvent> {
        self.0.borrow_mut().pop_front()
    }

    /// 递交一个节拍；队列里已有还没处理的节拍时合并成一个。
    fn push_tick(&self) {
        let mut queue = self.0.borrow_mut();
        if !queue.iter().any(|e| matches!(e, PlatformEvent::Tick)) {
            queue.push_back(PlatformEvent::Tick);
        }
    }

    /// 丢掉还没递交的遮罩输入：遮罩关闭后它们已经没有意义。
    pub(crate) fn discard_overlay_input(&self) {
        self.0
            .borrow_mut()
            .retain(|event| !matches!(event, PlatformEvent::OverlayInput(_)));
    }
}

impl WindowState {
    fn push(&self, event: PlatformEvent) {
        self.events.push_back(event);
    }

    fn pop(&self) -> Option<PlatformEvent> {
        self.events.pop_front()
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
            let (sender, sent) = mpsc::channel();
            let state = Box::new(WindowState {
                events: EventQueue::default(),
                sent,
                tray: RefCell::new(None),
                menu: MenuItems::default(),
                balloon_click: PendingClick::default(),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                shared: Rc::default(),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const WindowState as isize);
            Ok(Self {
                hwnd,
                state,
                sender,
            })
        }
    }

    /// 供其他线程向本循环投递事件的句柄。
    pub fn sender(&self) -> EventSender {
        EventSender {
            hwnd: self.hwnd.0 as isize,
            events: self.sender.clone(),
        }
    }

    /// 显示托盘图标；右键或左键单击弹出 [`tray_menu`](Self::tray_menu) 设置的菜单。
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

    /// 托盘图标的右键菜单；选中某项时本循环递交 [`PlatformEvent::TrayMenuItemSelected`]。
    pub fn tray_menu(&self) -> WindowsTrayMenu {
        WindowsTrayMenu {
            items: self.state.menu.clone(),
        }
    }

    /// 绑定到本窗口的全局热键；按下时本循环递交 [`PlatformEvent::HotkeyPressed`]。
    pub fn hotkeys(&self) -> WindowsHotkeys {
        WindowsHotkeys::new(self.hwnd)
    }

    /// 用本循环的托盘图标显示气泡通知；需要先 [`show_tray_icon`](Self::show_tray_icon)。
    /// 用户点击气泡时，本循环执行通知的点击操作。
    pub fn notifier(&self) -> TrayNotifier {
        TrayNotifier::new(self.hwnd, self.state.balloon_click.clone())
    }

    /// 位于 `path` 的配置文件；开始监视后，本循环递交 [`PlatformEvent::ConfigFileChanged`]。
    pub fn config_file(&self, path: PathBuf) -> WindowsConfigFile {
        WindowsConfigFile::new(path, self.state.shared.clone())
    }

    /// 系统截图工具对 PrtSc 键的占用；开始监视后，本循环递交
    /// [`PlatformEvent::PrintScreenKeySettingChanged`]。
    pub fn system_screenshot_tool(&self) -> WindowsScreenshotTool {
        WindowsScreenshotTool::new(self.state.shared.clone())
    }

    /// 选区遮罩；用户在遮罩上的输入由本循环以 [`PlatformEvent::OverlayInput`] 递交。
    ///
    /// 遮罩窗口和 D3D11 / Direct2D 设备在这里就建好并预热（约几百毫秒），之后每次打开遮罩不再新建。
    pub fn selection_overlay(&self) -> D2dSelectionOverlay {
        D2dSelectionOverlay::new(self.state.events.clone())
    }

    /// 节拍；开始后本循环按周期递交 [`PlatformEvent::Tick`]。
    pub fn ticker(&self) -> WindowsTicker {
        WindowsTicker::new(self.state.shared.clone())
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 运行消息循环，把每个事件交给 `handler`，直到它返回 `Break`。
    pub fn run(&self, mut handler: impl FnMut(PlatformEvent) -> ControlFlow<()>) {
        loop {
            while let Some(event) = self.state.pop() {
                if handler(event).is_break() {
                    return;
                }
            }
            if !self.pump(None) {
                return;
            }
        }
    }

    /// 处理消息直到出现一个事件或超时，供冒烟测试使用。
    pub fn wait_event(&self, timeout: Duration) -> Option<PlatformEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.state.pop() {
                return Some(event);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || !self.pump(Some(remaining)) {
                return None;
            }
        }
    }

    /// 等到有消息、配置目录或 PrtSc 键系统设置变化（或超时），然后处理掉它们；
    /// 收到 `WM_QUIT` 时返回 `false`。
    fn pump(&self, timeout: Option<Duration>) -> bool {
        let shared = &self.state.shared;
        let config_watch: Option<HANDLE> = shared.config_watch.borrow().as_ref().map(|w| w.0);
        let keyboard_watch: Option<HANDLE> =
            shared.keyboard_watch.borrow().as_ref().map(|w| w.handle());
        let ticker: Option<HANDLE> = shared.ticker.borrow().as_ref().map(|t| t.handle());
        let handles: Vec<HANDLE> = config_watch
            .into_iter()
            .chain(keyboard_watch)
            .chain(ticker)
            .collect();
        // INFINITE 是 u32::MAX；有限的等待时间不能恰好等于它。
        let millis = timeout.map_or(u32::MAX, |t| {
            t.as_millis().min(u128::from(u32::MAX - 1)) as u32
        });
        let woken = unsafe {
            MsgWaitForMultipleObjectsEx(Some(&handles), millis, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        let signaled = woken
            .0
            .checked_sub(WAIT_OBJECT_0.0)
            .and_then(|i| handles.get(i as usize).copied());
        if signaled.is_some() && signaled == ticker {
            // 自动重置的定时器：这次等待已经把它复位了。
            self.state.events.push_tick();
        } else if let Some(handle) = signaled
            && signaled == config_watch
        {
            // 重新布防，再用定时器合并编辑器保存时的连续多次变化。
            let _ = unsafe { FindNextChangeNotification(handle) };
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    CONFIG_SETTLE_TIMER,
                    CONFIG_SETTLE_DELAY_MS,
                    None,
                )
            };
        } else if signaled.is_some() && signaled == keyboard_watch {
            if let Some(watch) = shared.keyboard_watch.borrow().as_ref()
                && watch.rearm().is_err()
            {
                eprintln!("重新监视 PrtSc 键的系统设置失败");
            }
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    KEYBOARD_SETTLE_TIMER,
                    KEYBOARD_SETTLE_DELAY_MS,
                    None,
                )
            };
        }
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
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
                if mouse == NIN_BALLOONUSERCLICK {
                    // 先取出再执行，ShellExecute 期间可能重入窗口过程。
                    let click = state.balloon_click.borrow_mut().take();
                    if let Some(click) = click {
                        open_click_target(hwnd, &click);
                    }
                } else if mouse == WM_RBUTTONUP || mouse == WM_LBUTTONUP {
                    // 弹出菜单期间会重入窗口过程，先复制一份菜单内容。
                    let items = state.menu.borrow().clone();
                    if let Some(id) = show_tray_menu(hwnd, &items) {
                        state.push(PlatformEvent::TrayMenuItemSelected(id));
                    }
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == CONFIG_SETTLE_TIMER => {
                let _ = unsafe { KillTimer(Some(hwnd), CONFIG_SETTLE_TIMER) };
                state.push(PlatformEvent::ConfigFileChanged);
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == KEYBOARD_SETTLE_TIMER => {
                let _ = unsafe { KillTimer(Some(hwnd), KEYBOARD_SETTLE_TIMER) };
                state.push(PlatformEvent::PrintScreenKeySettingChanged);
                return LRESULT(0);
            }
            WM_CLOSE => {
                state.push(PlatformEvent::ExitRequested);
                return LRESULT(0);
            }
            WM_SENT_EVENT => {
                while let Ok(event) = state.sent.try_recv() {
                    state.push(event);
                }
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
fn show_tray_menu(hwnd: HWND, items: &[TrayMenuItem]) -> Option<TrayMenuItemId> {
    if items.is_empty() {
        return None;
    }
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (index, item) in items.iter().enumerate() {
            let label = HSTRING::from(item.label.as_str());
            // 菜单命令号从 1 开始（0 表示没有选中）。
            let _ = AppendMenuW(menu, MF_STRING, index + 1, &label);
        }
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
        let chosen = usize::try_from(command.0).ok()?.checked_sub(1)?;
        items.get(chosen).map(|item| item.id)
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
