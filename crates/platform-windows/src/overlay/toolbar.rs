//! 录屏控制条与区域边框的 Windows 实现（#19）。
//!
//! - 控制条：一个只有控制条大小的透明 DirectComposition 窗口（复用 [`LiveWindow`]），
//!   `WDA_EXCLUDEFROMCAPTURE` 保证不进任何屏幕捕获（包括本程序的录屏），`WS_EX_NOACTIVATE`
//!   保证点它不抢键盘焦点。窗口范围以外不属于它，鼠标照常落到底下的窗口。
//! - 区域边框：围住录制范围的四条细窗口（上下左右各一条），同样排除在捕获以外；
//!   `HTTRANSPARENT` + `WS_EX_TRANSPARENT`，不拦截鼠标。
//!
//! D3D11 / Direct2D 设备只在录屏期间持有（常驻实例的内存预算不为它常驻），录制开始时新建、
//! 停止时释放；录屏开始前的遮罩预热不受影响。

use std::cell::Cell;

use bps_platform::{
    OverlayCursor, OverlayScene, PlatformError, RecordingToolbar, RecordingToolbarView, Rect,
    RegionBorder,
};
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, InvalidateRect,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, HTTRANSPARENT, HWND_TOPMOST, RegisterClassExW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
    WDA_EXCLUDEFROMCAPTURE, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::gpu::Gpu;
use super::live::LiveWindow;
use crate::error::win_error;
use crate::event_loop::EventQueue;

const BORDER_CLASS: PCWSTR = w!("BetterPrtSc.RegionBorder");

/// 录屏控制条与区域边框。由 [`crate::EventLoop::recording_toolbar`] 创建，必须在事件循环的线程上使用。
pub struct D2dRecordingToolbar {
    queue: EventQueue,
    shown: Option<Shown>,
}

/// 显示期间的资源。
struct Shown {
    gpu: Gpu,
    window: LiveWindow,
    /// 控制条窗口当前的范围；变了就重建窗口。
    bounds: Rect,
    border: Option<Border>,
}

impl D2dRecordingToolbar {
    pub(crate) fn new(queue: EventQueue) -> Self {
        Self { queue, shown: None }
    }
}

impl RecordingToolbar for D2dRecordingToolbar {
    fn show(&mut self, view: &RecordingToolbarView) -> Result<(), PlatformError> {
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.bounds != view.bounds)
        {
            self.hide();
        }
        if self.shown.is_none() {
            let gpu = Gpu::new()?;
            let window = LiveWindow::new(&gpu, view.bounds, self.queue.clone(), false)?;
            self.shown = Some(Shown {
                gpu,
                window,
                bounds: view.bounds,
                border: None,
            });
        }
        let shown = self.shown.as_mut().expect("刚刚确保过");
        let scene = OverlayScene {
            items: view.items.clone(),
            cursor: OverlayCursor::Arrow,
        };
        let result = shown.window.render(&shown.gpu, None, &scene);
        let border = shown.set_border(view.border);
        if result.is_err() || border.is_err() {
            self.hide();
        }
        result.and(border)
    }

    fn hide(&mut self) {
        if self.shown.take().is_some() {
            // 隐藏前已经排进队列的输入不再递交。
            self.queue.discard_overlay_input();
        }
    }
}

impl Shown {
    fn set_border(&mut self, border: Option<RegionBorder>) -> Result<(), PlatformError> {
        match (border, self.border.as_mut()) {
            (None, _) => self.border = None,
            (Some(border), Some(windows)) => windows.update(border)?,
            (Some(border), None) => self.border = Some(Border::new(border)?),
        }
        Ok(())
    }
}

/// 围住区域的四条边。
struct Border {
    edges: [HWND; 4],
    current: Cell<Option<RegionBorder>>,
}

impl Border {
    fn new(border: RegionBorder) -> Result<Self, PlatformError> {
        register_border_class()?;
        let mut edges = Vec::new();
        for _ in 0..4 {
            match create_edge() {
                Ok(hwnd) => edges.push(hwnd),
                Err(e) => {
                    edges.into_iter().for_each(destroy);
                    return Err(e);
                }
            }
        }
        let this = Self {
            edges: edges.try_into().expect("四条边"),
            current: Cell::new(None),
        };
        this.update(border)?;
        Ok(this)
    }

    fn update(&self, border: RegionBorder) -> Result<(), PlatformError> {
        if self.current.get() == Some(border) {
            return Ok(());
        }
        let color_changed = self.current.get().map(|b| b.color) != Some(border.color);
        let (r, w) = (border.rect, border.width as i32);
        let (width, height) = (r.width as i32, r.height as i32);
        let rects = [
            (r.x - w, r.y - w, width + 2 * w, w),
            (r.x - w, r.y + height, width + 2 * w, w),
            (r.x - w, r.y, w, height),
            (r.x + width, r.y, w, height),
        ];
        for (hwnd, (x, y, cx, cy)) in self.edges.iter().zip(rects) {
            unsafe {
                if color_changed {
                    let c = border.color;
                    let colorref = u32::from(c.r) | u32::from(c.g) << 8 | u32::from(c.b) << 16;
                    SetWindowLongPtrW(*hwnd, GWLP_USERDATA, colorref as isize);
                    let _ = InvalidateRect(Some(*hwnd), None, false);
                }
                SetWindowPos(
                    *hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    cx,
                    cy,
                    SWP_SHOWWINDOW | SWP_NOACTIVATE,
                )
            }
            .map_err(|e| win_error("显示区域边框失败", e))?;
        }
        self.current.set(Some(border));
        Ok(())
    }
}

impl Drop for Border {
    fn drop(&mut self) {
        self.edges.into_iter().for_each(destroy);
    }
}

fn destroy(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn register_border_class() -> Result<(), PlatformError> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| win_error("读取模块句柄失败", e))?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(border_proc),
            hInstance: module.into(),
            lpszClassName: BORDER_CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(PlatformError::new("注册区域边框窗口类失败"));
        }
    }
    Ok(())
}

fn create_edge() -> Result<HWND, PlatformError> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| win_error("读取模块句柄失败", e))?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            BORDER_CLASS,
            w!("BetterPrtSc 录屏区域边框"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| win_error("创建区域边框窗口失败", e))?;
        // 低于 Windows 10 2004 时不支持，边框会出现在录屏里。
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        Ok(hwnd)
    }
}

/// 边框窗口过程：整个窗口涂成 `GWLP_USERDATA` 里的 COLORREF，鼠标穿透。
unsafe extern "system" fn border_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => unsafe {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut rect = Default::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let brush = CreateSolidBrush(COLORREF(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32));
            FillRect(hdc, &rect, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
