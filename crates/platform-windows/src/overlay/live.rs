//! 实时画面上的遮罩窗口（[`Backdrop::Live`](bps_platform::Backdrop::Live)，录屏框选用）。
//!
//! 和冻结帧遮罩共用窗口过程、GPU 资源和绘制代码，区别是：
//! - 不画冻结帧，交换链是预乘 alpha 的 DirectComposition 交换链，没画到的地方透明，
//!   底下的桌面照常变化；
//! - 窗口设为 `WDA_EXCLUDEFROMCAPTURE`，不出现在任何屏幕捕获里（包括本程序的录屏），
//!   所以录屏时遮罩上的边框、控制条不会被录进去（Windows 10 2004 起）；
//! - 每次打开时新建、关闭时销毁，不常驻（交换链和合成资源只在框选期间占用显存）。
//!
//! 输入照常由共用的窗口过程转换成 `OverlayInput`；没有 `WS_EX_LAYERED` / `WS_EX_TRANSPARENT`，
//! 透明的地方也接收鼠标。

use std::cell::Cell;
use std::rc::Rc;

use bps_platform::{OverlayCursor, OverlayScene, PlatformError, Point, Rect};
use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    ID2D1Bitmap1,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DestroyWindow, GWLP_USERDATA, HWND_TOPMOST, RegisterClassExW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowDisplayAffinity,
    SetWindowLongPtrW, SetWindowPos, WDA_EXCLUDEFROMCAPTURE, WNDCLASSEXW,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Interface, PCWSTR, w};

use super::draw;
use super::gpu::Gpu;
use super::window::{PartialWindow, Shared, apply_cursor, cloak, window_proc};
use crate::error::win_error;
use crate::event_loop::EventQueue;

/// 实时遮罩窗口的类名。
const CLASS_NAME: PCWSTR = w!("BetterPrtSc.LiveOverlay");

pub(super) struct LiveWindow {
    hwnd: HWND,
    shared: Rc<Shared>,
    swap_chain: IDXGISwapChain1,
    /// 交换链后台缓冲区上的 D2D 渲染目标。
    target: ID2D1Bitmap1,
    _composition: IDCompositionDevice,
    _composition_target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    bounds: Rect,
    /// 已经显示出来了（第一次 `render` 之后）。
    shown: Cell<bool>,
}

impl LiveWindow {
    /// 创建覆盖 `bounds`（虚拟桌面物理像素）的透明遮罩窗口，cloak 状态，还不显示。
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
                return Err(PlatformError::new("注册实时遮罩窗口类失败"));
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                CLASS_NAME,
                w!("BetterPrtSc 录屏选区"),
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
            .map_err(|e| win_error("创建实时遮罩窗口失败", e))?;
            cloak(hwnd, true);
            // 低于 Windows 10 2004 时不支持，遮罩仍可用（录屏开始前遮罩已关闭）。
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            let shared = Rc::new(Shared {
                queue,
                origin: Cell::new(Point::new(bounds.x, bounds.y)),
                cursor: Cell::new(OverlayCursor::Crosshair),
                active: Cell::new(false),
                stale: Cell::new(false),
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
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
                ..Default::default()
            };
            let swap_chain = factory
                .CreateSwapChainForComposition(gpu.render_device(), &desc, None)
                .map_err(|e| {
                    win_error("创建实时遮罩交换链失败（虚拟桌面宽高不能超过 16384）", e)
                })?;
            let dxgi_device: IDXGIDevice = gpu
                .render_device()
                .cast()
                .map_err(|e| win_error("读取 DXGI 设备失败", e))?;
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)
                .map_err(|e| win_error("创建 DirectComposition 设备失败", e))?;
            let composition_target = composition
                .CreateTargetForHwnd(hwnd, true)
                .map_err(|e| win_error("创建 DirectComposition 目标失败", e))?;
            let visual = composition
                .CreateVisual()
                .map_err(|e| win_error("创建 DirectComposition 视觉对象失败", e))?;
            visual
                .SetContent(&swap_chain)
                .and_then(|()| composition_target.SetRoot(&visual))
                .and_then(|()| composition.Commit())
                .map_err(|e| win_error("设置 DirectComposition 内容失败", e))?;
            let surface: IDXGISurface = swap_chain
                .GetBuffer(0)
                .map_err(|e| win_error("读取交换链缓冲区失败", e))?;
            let target = gpu
                .d2d
                .CreateBitmapFromDxgiSurface(
                    &surface,
                    Some(&D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                        ..Default::default()
                    }),
                )
                .map_err(|e| win_error("创建实时遮罩渲染目标失败", e))?;
            std::mem::forget(partial);
            Ok(Self {
                hwnd,
                shared,
                swap_chain,
                target,
                _composition: composition,
                _composition_target: composition_target,
                _visual: visual,
                bounds,
                shown: Cell::new(false),
            })
        }
    }

    /// 按 `scene` 画一帧并呈现；第一次调用时把窗口显示出来并取得键盘焦点。
    pub fn render(&self, gpu: &Gpu, scene: &OverlayScene) -> Result<(), PlatformError> {
        if self.shared.cursor.replace(scene.cursor) != scene.cursor && self.shared.active.get() {
            apply_cursor(scene.cursor);
        }
        let first = !self.shown.get();
        if first {
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
            .map_err(|e| win_error("显示实时遮罩窗口失败", e))?;
        }
        let origin = Point::new(self.bounds.x, self.bounds.y);
        draw::draw_transparent(gpu, &self.target, origin, scene)?;
        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|e| win_error("呈现实时遮罩失败", e))?;
        if first {
            // 与冻结帧遮罩相同：等 DWM 合成完第一帧再取消 cloak，最后抢前台取得键盘焦点。
            unsafe {
                let _ = DwmFlush();
                self.shared.active.set(true);
                cloak(self.hwnd, false);
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
            apply_cursor(self.shared.cursor.get());
            self.shown.set(true);
        }
        Ok(())
    }
}

impl Drop for LiveWindow {
    fn drop(&mut self) {
        self.shared.active.set(false);
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
