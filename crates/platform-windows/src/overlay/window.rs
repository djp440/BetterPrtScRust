//! 遮罩窗口：铺满虚拟桌面的置顶 `WS_POPUP` 窗口 + flip-discard 交换链。
//!
//! 窗口常驻：平时隐藏并 cloak，打开遮罩时按 #2 原型实测的顺序显示（见 [`OverlayWindow::reveal`]），
//! 关闭时只隐藏、不销毁。窗口过程把鼠标和按键转换成 [`OverlayInput`] 放进事件循环的队列。

use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;

use bps_platform::{
    Modifiers, OverlayCursor, OverlayInput, OverlayKey, PlatformError, PlatformEvent, Point,
    PointerButton, Rect,
};
use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, ID2D1Bitmap1,
};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmFlush, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory2, IDXGISurface,
    IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_DIFF, SetWindowRgn, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, CPS_CANCEL, HIMC, IACE_DEFAULT,
    ImmAssociateContextEx, ImmGetContext, ImmNotifyIME, ImmReleaseContext, ImmSetCandidateWindow,
    ImmSetCompositionWindow, NI_COMPOSITIONSTR,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_BACK, VK_C,
    VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_LWIN, VK_MENU, VK_RETURN, VK_RIGHT, VK_RWIN,
    VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetCursorPos,
    GetWindowLongPtrW, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_CROSS, IDC_IBEAM, IDC_SIZEALL,
    IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW, RegisterClassExW, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursor, SetForegroundWindow, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_CHAR,
    WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDBLCLK,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

use super::gpu::{Gpu, bitmap_properties};
use crate::error::win_error;
use crate::event_loop::EventQueue;

/// 遮罩窗口的类名。
const CLASS_NAME: PCWSTR = w!("BetterPrtSc.SelectionOverlay");

thread_local! {
    /// 本线程上正显示着的遮罩窗口，供文件对话框作为所有者窗口（对话框因此显示在遮罩之上）。
    static VISIBLE: Cell<Option<isize>> = const { Cell::new(None) };
}

/// 本线程上正显示着的遮罩窗口。
pub(crate) fn visible_overlay_window() -> Option<HWND> {
    VISIBLE
        .with(Cell::get)
        .map(|hwnd| HWND(hwnd as *mut c_void))
}

/// 窗口过程与 [`OverlayWindow`] 共享的状态；窗口过程会被同步重入，所以只用 `Cell`。
pub(super) struct Shared {
    pub queue: EventQueue,
    /// 窗口左上角在虚拟桌面中的位置：客户区坐标 + `origin` = 虚拟桌面坐标。
    pub origin: Cell<Point>,
    pub cursor: Cell<OverlayCursor>,
    /// 遮罩正显示着；只在这期间递交用户输入。
    pub active: Cell<bool>,
    /// 显示器布局变了，窗口和交换链需要重建。
    pub stale: Cell<bool>,
    /// `WM_CHAR` 递交的 UTF-16 高位代理项，等低位代理项到了再拼成一个字符。
    pub high_surrogate: Cell<Option<u16>>,
}

pub(crate) struct OverlayWindow {
    hwnd: HWND,
    shared: Rc<Shared>,
    swap_chain: IDXGISwapChain1,
    /// 交换链后台缓冲区上的 D2D 渲染目标。
    pub target: ID2D1Bitmap1,
    bounds: Rect,
    /// 正在输入文字时的插入点（虚拟桌面物理像素）；`None` 时输入法关闭。
    text_caret: Cell<Option<Rect>>,
}

impl OverlayWindow {
    /// 创建覆盖 `bounds`（虚拟桌面物理像素）的遮罩窗口，隐藏并 cloak。
    pub fn new(gpu: &Gpu, bounds: Rect, queue: EventQueue) -> Result<Self, PlatformError> {
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| win_error("读取模块句柄失败", e))?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(window_proc),
                hInstance: module.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(PlatformError::new("注册遮罩窗口类失败"));
            }
            // NOREDIRECTIONBITMAP：交换链窗口不需要 GDI 重定向表面，4K 下省 30 MB 以上。
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                CLASS_NAME,
                w!("BetterPrtSc 选区遮罩"),
                WS_POPUP,
                bounds.x,
                bounds.y,
                bounds.width as i32,
                bounds.height as i32,
                None,
                None,
                Some(module.into()),
                None,
            )
            .map_err(|e| win_error("创建遮罩窗口失败", e))?;
            cloak(hwnd, true);
            // 平时不接受文字输入：关掉输入法，打字不会弹出输入法的窗口。
            let _ = ImmAssociateContextEx(hwnd, HIMC::default(), 0);
            let shared = Rc::new(Shared {
                queue,
                origin: Cell::new(Point::new(bounds.x, bounds.y)),
                cursor: Cell::new(OverlayCursor::Crosshair),
                active: Cell::new(false),
                stale: Cell::new(false),
                high_surrogate: Cell::new(None),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&shared) as isize);
            // 从这里起出错时由 Drop 销毁窗口。
            let partial = PartialWindow(hwnd);

            let factory: IDXGIFactory2 = gpu
                .render_adapter()
                .GetParent()
                .map_err(|e| win_error("读取 DXGI 工厂失败", e))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: bounds.width,
                Height: bounds.height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
                ..Default::default()
            };
            let swap_chain = factory
                .CreateSwapChainForHwnd(gpu.render_device(), hwnd, &desc, None, None)
                .map_err(|e| win_error("创建遮罩交换链失败（虚拟桌面宽高不能超过 16384）", e))?;
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            let surface: IDXGISurface = swap_chain
                .GetBuffer(0)
                .map_err(|e| win_error("读取交换链缓冲区失败", e))?;
            let target = gpu
                .d2d
                .CreateBitmapFromDxgiSurface(
                    &surface,
                    Some(&bitmap_properties(
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    )),
                )
                .map_err(|e| win_error("创建遮罩渲染目标失败", e))?;
            std::mem::forget(partial);
            Ok(Self {
                hwnd,
                shared,
                swap_chain,
                target,
                bounds,
                text_caret: Cell::new(None),
            })
        }
    }

    /// 窗口覆盖的范围（虚拟桌面物理像素）。
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 收到过 `WM_DISPLAYCHANGE`，需要重建。
    pub fn is_stale(&self) -> bool {
        self.shared.stale.get()
    }

    /// 让 `area`（虚拟桌面物理像素）透过窗口：用窗口区域把这块挖空，底下的桌面直接露出来、
    /// 鼠标事件也落到底下的窗口；同时让窗口不出现在屏幕捕获里。`None` 恢复整个窗口。
    pub fn set_pass_through(&self, area: Option<Rect>) -> Result<(), PlatformError> {
        unsafe {
            let Some(area) = area else {
                SetWindowRgn(self.hwnd, None, true);
                let _ = SetWindowDisplayAffinity(self.hwnd, WDA_NONE);
                return Ok(());
            };
            let bounds = self.bounds;
            let whole = CreateRectRgn(0, 0, bounds.width as i32, bounds.height as i32);
            let left = area.x - bounds.x;
            let top = area.y - bounds.y;
            let hole = CreateRectRgn(
                left,
                top,
                left + area.width as i32,
                top + area.height as i32,
            );
            let combined = CombineRgn(Some(whole), Some(whole), Some(hole), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(hole.0));
            if combined.0 == 0 {
                let _ = DeleteObject(HGDIOBJ(whole.0));
                return Err(PlatformError::new("计算遮罩透过范围失败"));
            }
            // 成功后区域归系统所有。
            if SetWindowRgn(self.hwnd, Some(whole), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(whole.0));
                return Err(PlatformError::new("设置遮罩透过范围失败"));
            }
            SetWindowDisplayAffinity(self.hwnd, WDA_EXCLUDEFROMCAPTURE)
                .map_err(|e| win_error("设置遮罩不被捕获失败", e))
        }
    }

    pub fn set_cursor(&self, cursor: OverlayCursor) {
        if self.shared.cursor.replace(cursor) != cursor && self.shared.active.get() {
            apply_cursor(cursor);
        }
    }

    /// 开始、继续或结束文字输入：`Some` 时打开输入法，把组字窗口放在插入点、候选窗口避开
    /// 插入点所在的那一行；`None` 时取消没完成的组字并关闭输入法。
    pub fn set_text_caret(&self, caret: Option<Rect>) {
        if self.text_caret.replace(caret) == caret {
            return;
        }
        let hwnd = self.hwnd;
        unsafe {
            let Some(caret) = caret else {
                let context = ImmGetContext(hwnd);
                if !context.is_invalid() {
                    let _ = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
                    let _ = ImmReleaseContext(hwnd, context);
                }
                let _ = ImmAssociateContextEx(hwnd, HIMC::default(), 0);
                return;
            };
            let _ = ImmAssociateContextEx(hwnd, HIMC::default(), IACE_DEFAULT);
            let context = ImmGetContext(hwnd);
            if context.is_invalid() {
                return;
            }
            let left = caret.x - self.bounds.x;
            let top = caret.y - self.bounds.y;
            let line = RECT {
                left,
                top,
                right: left + caret.width as i32,
                bottom: top + caret.height as i32,
            };
            let _ = ImmSetCompositionWindow(
                context,
                &COMPOSITIONFORM {
                    dwStyle: CFS_POINT,
                    ptCurrentPos: POINT { x: left, y: top },
                    rcArea: RECT::default(),
                },
            );
            let _ = ImmSetCandidateWindow(
                context,
                &CANDIDATEFORM {
                    dwIndex: 0,
                    dwStyle: CFS_EXCLUDE,
                    ptCurrentPos: POINT {
                        x: left,
                        y: line.bottom,
                    },
                    rcArea: line,
                },
            );
            let _ = ImmReleaseContext(hwnd, context);
        }
    }

    /// 把画好的一帧呈现出来。
    pub fn present(&self) -> Result<(), PlatformError> {
        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|e| win_error("呈现遮罩画面失败", e))
    }

    /// 显示第一帧之前：以 cloak 状态显示并置顶窗口（用户还看不到）。
    pub fn place_cloaked(&self) -> Result<(), PlatformError> {
        let b = self.bounds;
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                b.x,
                b.y,
                b.width as i32,
                b.height as i32,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| win_error("显示遮罩窗口失败", e))
    }

    /// 第一帧已呈现之后：等 DWM 合成完这一帧再取消 cloak（否则约 2% 的触发会先闪出上一次的画面），
    /// 最后抢前台取得键盘焦点（抢前台要 10–50 ms，放在最后）。
    pub fn reveal(&self) {
        unsafe {
            let _ = DwmFlush();
            self.shared.active.set(true);
            VISIBLE.with(|v| v.set(Some(self.hwnd.0 as isize)));
            cloak(self.hwnd, false);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        apply_cursor(self.shared.cursor.get());
    }

    /// 隐藏窗口（不销毁），停止递交输入，关闭输入法。
    pub fn hide(&self) {
        self.shared.active.set(false);
        self.shared.high_surrogate.set(None);
        self.set_text_caret(None);
        VISIBLE.with(|v| v.set(None));
        unsafe {
            if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
            SetWindowRgn(self.hwnd, None, false);
            let _ = SetWindowDisplayAffinity(self.hwnd, WDA_NONE);
            cloak(self.hwnd, true);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        if visible_overlay_window() == Some(self.hwnd) {
            VISIBLE.with(|v| v.set(None));
        }
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 创建过程中出错时销毁已建好的窗口。
pub(super) struct PartialWindow(pub HWND);

impl Drop for PartialWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.0, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.0);
        }
    }
}

pub(super) fn cloak(hwnd: HWND, on: bool) {
    let value = BOOL::from(on);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const BOOL as *const c_void,
            size_of::<BOOL>() as u32,
        );
    }
}

pub(super) fn apply_cursor(cursor: OverlayCursor) {
    let id = match cursor {
        OverlayCursor::Crosshair => IDC_CROSS,
        OverlayCursor::Arrow => IDC_ARROW,
        OverlayCursor::ResizeEw => IDC_SIZEWE,
        OverlayCursor::ResizeNs => IDC_SIZENS,
        OverlayCursor::ResizeNwse => IDC_SIZENWSE,
        OverlayCursor::ResizeNesw => IDC_SIZENESW,
        OverlayCursor::Move => IDC_SIZEALL,
        OverlayCursor::IBeam => IDC_IBEAM,
    };
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, id) {
            SetCursor(Some(cursor));
        }
    }
}

pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let shared = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Shared;
    let Some(shared) = (unsafe { shared.as_ref() }) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    let active = shared.active.get();
    let input = |input: OverlayInput| {
        if active {
            shared.queue.push_back(PlatformEvent::OverlayInput(input));
        }
        LRESULT(0)
    };
    let position = || {
        let origin = shared.origin.get();
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        Point::new(origin.x + x, origin.y + y)
    };
    let pressed = |button, double_click| {
        unsafe { SetCapture(hwnd) };
        input(OverlayInput::PointerPressed {
            button,
            position: position(),
            modifiers: modifiers(),
            double_click,
        })
    };
    let released = |button| {
        if button == PointerButton::Left {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        input(OverlayInput::PointerReleased {
            button,
            position: position(),
            modifiers: modifiers(),
        })
    };
    match msg {
        WM_MOUSEMOVE => input(OverlayInput::PointerMoved {
            position: position(),
            modifiers: modifiers(),
        }),
        WM_LBUTTONDOWN => pressed(PointerButton::Left, false),
        WM_LBUTTONDBLCLK => pressed(PointerButton::Left, true),
        WM_LBUTTONUP => released(PointerButton::Left),
        WM_RBUTTONDOWN => pressed(PointerButton::Right, false),
        WM_RBUTTONDBLCLK => pressed(PointerButton::Right, true),
        WM_RBUTTONUP => released(PointerButton::Right),
        // 修饰键按下或松开时指针没动，也在原位置递交一次移动：依赖修饰键的交互（如按住 Ctrl
        // 临时不吸附）不必等用户再动鼠标就能更新。按住不放时的自动重复不递交。
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP if is_modifier(wparam) => {
            let repeat = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) && lparam.0 & (1 << 30) != 0;
            if !repeat && let Some(position) = cursor_position() {
                input(OverlayInput::PointerMoved {
                    position,
                    modifiers: modifiers(),
                });
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => match key(wparam) {
            Some(key) => input(OverlayInput::KeyPressed {
                key,
                modifiers: modifiers(),
            }),
            None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },
        // 键盘直接输入和输入法确认的文字（DefWindowProc 把 WM_IME_CHAR 转成 WM_CHAR）。
        WM_CHAR => match character(&shared.high_surrogate, wparam.0 as u16) {
            Some(c) if !c.is_control() => input(OverlayInput::Character(c)),
            _ => LRESULT(0),
        },
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            apply_cursor(shared.cursor.get());
            LRESULT(1)
        }
        // 横跨多块缩放不同的显示器时系统会建议新的窗口尺寸；遮罩必须保持物理像素尺寸，忽略。
        WM_DPICHANGED => LRESULT(0),
        WM_DISPLAYCHANGE => {
            shared.stale.set(true);
            input(OverlayInput::Interrupted)
        }
        // Alt+F4：当作中断，不销毁常驻的窗口。
        WM_CLOSE => input(OverlayInput::Interrupted),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn key(wparam: WPARAM) -> Option<OverlayKey> {
    match VIRTUAL_KEY(wparam.0 as u16) {
        VK_ESCAPE => Some(OverlayKey::Escape),
        VK_RETURN => Some(OverlayKey::Enter),
        VK_LEFT => Some(OverlayKey::Left),
        VK_RIGHT => Some(OverlayKey::Right),
        VK_UP => Some(OverlayKey::Up),
        VK_DOWN => Some(OverlayKey::Down),
        VK_C => Some(OverlayKey::C),
        VK_BACK => Some(OverlayKey::Backspace),
        _ => None,
    }
}

/// `WM_CHAR` 递交的一个 UTF-16 代码单元：高位代理项先存起来，与随后的低位代理项拼成一个
/// 字符；落单的代理项丢弃。
fn character(high_surrogate: &Cell<Option<u16>>, unit: u16) -> Option<char> {
    match unit {
        0xD800..=0xDBFF => {
            high_surrogate.set(Some(unit));
            None
        }
        0xDC00..=0xDFFF => {
            let high = high_surrogate.take()?;
            char::decode_utf16([high, unit]).next()?.ok()
        }
        _ => {
            high_surrogate.set(None);
            char::from_u32(u32::from(unit))
        }
    }
}

fn is_modifier(wparam: WPARAM) -> bool {
    matches!(
        VIRTUAL_KEY(wparam.0 as u16),
        VK_CONTROL | VK_MENU | VK_SHIFT | VK_LWIN | VK_RWIN
    )
}

/// 光标此刻在虚拟桌面上的位置（物理像素：进程是 PMv2 DPI 感知的）。
fn cursor_position() -> Option<Point> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some(Point::new(point.x, point.y))
}

fn modifiers() -> Modifiers {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    Modifiers {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        win: down(VK_LWIN) || down(VK_RWIN),
    }
}
