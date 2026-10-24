//! 一次性原型代码（ticket #2）：原生 Win32 + Direct2D 选区遮罩。
//!
//! 常驻进程注册全局热键 → 收到 WM_HOTKEY → DXGI Desktop Duplication 拍冻结帧（GPU 纹理，
//! 直接包成 D2D 位图，不经 CPU）→ 一个铺满虚拟桌面的 WS_POPUP 窗口（flip 模型交换链）显示
//! 冻结帧 + 半透明遮罩，可拖拽框选。只为回答 ADR 0003 的延迟问题，不要复用到正式代码。

#[path = "../../common/capture.rs"]
mod capture;
#[path = "../../common/proto.rs"]
mod proto;

use std::cell::RefCell;
use std::ffi::c_void;

use proto::{Args, Cmd, Strategy, Timeline, emit, qpc};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmFlush, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetCapture, SetFocus, UnregisterHotKey, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, PCWSTR, Result, w};

const WM_APP_CMD: u32 = WM_APP + 1;
const TIMER_AUTOCLOSE: usize = 1;
const TIMER_SAFETY: usize = 2;
const CLASS_NAME: PCWSTR = w!("BpsOverlayNativePrototype");

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn rectf(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

/// 渲染所需的 GPU 资源：D3D11 设备（与截屏共用）+ D2D + DirectWrite。
struct Gfx {
    cap: capture::Capturer,
    render_adapter: usize,
    dxgi_factory: IDXGIFactory2,
    ctx: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    formats: Vec<(u32, IDWriteTextFormat)>,
    mask: ID2D1SolidColorBrush,
    border: ID2D1SolidColorBrush,
    white: ID2D1SolidColorBrush,
    label_bg: ID2D1SolidColorBrush,
    sentinel: ID2D1SolidColorBrush,
}

impl Gfx {
    fn new() -> Result<Self> {
        unsafe {
            let cap = capture::Capturer::new()?;
            let render_adapter = cap.primary().map(|o| o.adapter_index).unwrap_or(0);
            let actx = &cap.adapters[render_adapter];
            let dxgi_device: IDXGIDevice = actx.device.cast()?;
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let ctx = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // 关键：DPI 固定 96，让 D2D 的 DIP 等于物理像素；缩放由我们按显示器自己算。
            ctx.SetDpi(96.0, 96.0);
            let dxgi_factory: IDXGIFactory2 = actx.adapter.GetParent()?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let brush = |c: D2D1_COLOR_F| ctx.CreateSolidColorBrush(&c, None);
            Ok(Self {
                mask: brush(color(0.0, 0.0, 0.0, 0.45))?,
                border: brush(color(0.12, 0.56, 1.0, 1.0))?,
                white: brush(color(1.0, 1.0, 1.0, 1.0))?,
                label_bg: brush(color(0.0, 0.0, 0.0, 0.7))?,
                sentinel: brush(color(1.0, 0.0, 1.0, 1.0))?,
                cap,
                render_adapter,
                dxgi_factory,
                ctx,
                dwrite,
                formats: Vec::new(),
            })
        }
    }

    fn device(&self) -> &ID3D11Device {
        &self.cap.adapters[self.render_adapter].device
    }

    /// 每个 DPI 一个文字格式：遮罩窗口横跨多块缩放不同的显示器，文字大小按所在显示器算。
    fn format(&mut self, dpi: u32) -> Result<IDWriteTextFormat> {
        if let Some((_, f)) = self.formats.iter().find(|(d, _)| *d == dpi) {
            return Ok(f.clone());
        }
        let f = unsafe {
            self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0 * dpi as f32 / 96.0,
                w!(""),
            )?
        };
        self.formats.push((dpi, f.clone()));
        Ok(f)
    }

    /// 冻结帧 → D2D 位图。同一适配器直接包 DXGI 表面（零拷贝），跨适配器读回 CPU 再上传。
    fn bitmaps(
        &self,
        frames: Vec<capture::GpuFrame>,
    ) -> Result<Vec<(capture::OutputInfo, ID2D1Bitmap1)>> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                // 桌面复制出来的 alpha 不可靠，必须忽略。
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let mut out = Vec::new();
        for f in frames {
            let bmp = unsafe {
                if f.info.adapter_index == self.render_adapter {
                    let surface: IDXGISurface = f.texture.cast()?;
                    self.ctx
                        .CreateBitmapFromDxgiSurface(&surface, Some(&props))?
                } else {
                    let data = self.cap.read_back(&f)?;
                    self.ctx.CreateBitmap(
                        D2D_SIZE_U {
                            width: f.width,
                            height: f.height,
                        },
                        Some(data.as_ptr() as *const c_void),
                        f.width * 4,
                        &props,
                    )?
                }
            };
            out.push((f.info, bmp));
        }
        Ok(out)
    }
}

/// 铺满虚拟桌面的遮罩窗口 + flip 模型交换链。
struct Win {
    hwnd: HWND,
    swap: IDXGISwapChain1,
    target: ID2D1Bitmap1,
    rect: RECT,
}

impl Win {
    fn new(gfx: &Gfx, rect: RECT) -> Result<Self> {
        unsafe {
            let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
            let hinst = GetModuleHandleW(None)?;
            // NOREDIRECTIONBITMAP：交换链窗口不需要 GDI 重定向表面，4K 下能省 30+ MB。
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                CLASS_NAME,
                w!("overlay-native"),
                WS_POPUP,
                rect.left,
                rect.top,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            )?;
            cloak(hwnd, true);
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w as u32,
                Height: h as u32,
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
            let swap =
                gfx.dxgi_factory
                    .CreateSwapChainForHwnd(gfx.device(), hwnd, &desc, None, None)?;
            let _ = gfx
                .dxgi_factory
                .MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let target = gfx
                .ctx
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            Ok(Self {
                hwnd,
                swap,
                target,
                rect,
            })
        }
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn cloak(hwnd: HWND, on: bool) {
    let v = BOOL::from(on);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &v as *const BOOL as *const c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

struct Session {
    n: u32,
    frames: Vec<(capture::OutputInfo, ID2D1Bitmap1)>,
    anchor: (i32, i32),
    sel: Option<RECT>,
    dragging: bool,
}

struct App {
    args: Args,
    gfx: Option<Gfx>,
    win: Option<Win>,
    session: Option<Session>,
    count: u32,
    hold_next: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// 画一帧：冻结帧 + 遮罩 + 选区边框 + 尺寸标签 + 哨兵方块。
fn render(gfx: &mut Gfx, win: &Win, s: &Session, sync: u32) -> Result<()> {
    let (ox, oy) = (win.rect.left, win.rect.top);
    let (w, h) = (
        (win.rect.right - win.rect.left) as f32,
        (win.rect.bottom - win.rect.top) as f32,
    );
    let label_fmt = match s.sel {
        Some(sel) => {
            let dpi = s
                .frames
                .iter()
                .find(|(o, _)| {
                    let (x, y) = (sel.left + ox, sel.top + oy);
                    x >= o.rect.left && x < o.rect.right && y >= o.rect.top && y < o.rect.bottom
                })
                .map(|(o, _)| o.dpi)
                .unwrap_or(96);
            Some((gfx.format(dpi)?, dpi))
        }
        None => None,
    };
    unsafe {
        let ctx = &gfx.ctx;
        ctx.SetTarget(&win.target);
        ctx.BeginDraw();
        ctx.Clear(Some(&color(0.0, 0.0, 0.0, 1.0)));
        for (o, bmp) in &s.frames {
            let dst = rectf(
                (o.rect.left - ox) as f32,
                (o.rect.top - oy) as f32,
                (o.rect.right - ox) as f32,
                (o.rect.bottom - oy) as f32,
            );
            ctx.DrawBitmap(
                bmp,
                Some(&dst),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        match s.sel {
            None => ctx.FillRectangle(&rectf(0.0, 0.0, w, h), &gfx.mask),
            Some(r) => {
                let (l, t, rr, b) = (r.left as f32, r.top as f32, r.right as f32, r.bottom as f32);
                ctx.FillRectangle(&rectf(0.0, 0.0, w, t), &gfx.mask);
                ctx.FillRectangle(&rectf(0.0, b, w, h), &gfx.mask);
                ctx.FillRectangle(&rectf(0.0, t, l, b), &gfx.mask);
                ctx.FillRectangle(&rectf(rr, t, w, b), &gfx.mask);
                if let Some((fmt, dpi)) = &label_fmt {
                    let scale = *dpi as f32 / 96.0;
                    ctx.DrawRectangle(
                        &rectf(l - 0.5, t - 0.5, rr + 0.5, b + 0.5),
                        &gfx.border,
                        scale.round().max(1.0),
                        None,
                    );
                    let text: Vec<u16> = format!("{} × {}", r.right - r.left, r.bottom - r.top)
                        .encode_utf16()
                        .collect();
                    let lh = 22.0 * scale;
                    let ty = if t - lh - 4.0 * scale >= 0.0 {
                        t - lh - 4.0 * scale
                    } else {
                        t + 4.0 * scale
                    };
                    let lr = rectf(l, ty, l + 110.0 * scale, ty + lh);
                    ctx.FillRectangle(&lr, &gfx.label_bg);
                    ctx.DrawText(
                        &text,
                        fmt,
                        &rectf(
                            lr.left + 6.0 * scale,
                            lr.top + 3.0 * scale,
                            lr.right,
                            lr.bottom,
                        ),
                        &gfx.white,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
        }
        // 哨兵：主显示器 (0,0) 处 8×8 品红方块，绿色分量编码触发序号。
        let code = proto::sentinel_code(s.n) as f32 / 255.0;
        gfx.sentinel.SetColor(&color(1.0, code, 1.0, 1.0));
        let (sx, sy) = ((-ox) as f32, (-oy) as f32);
        let sz = proto::SENTINEL_SIZE as f32;
        ctx.FillRectangle(&rectf(sx, sy, sx + sz, sy + sz), &gfx.sentinel);
        ctx.EndDraw(None, None)?;
        ctx.SetTarget(None);
        win.swap.Present(sync, DXGI_PRESENT(0)).ok()?;
    }
    Ok(())
}

fn trigger(app: &mut App, t_hotkey: i64) -> Result<()> {
    if app.session.is_some() {
        return Ok(());
    }
    app.count += 1;
    let n = app.count;
    let mut tl = Timeline::new(n, t_hotkey);
    tl.mark("dispatch");
    // 显示器拓扑变了就整体重建（Capturer 的 DXGI factory 会失效）。
    if app.gfx.as_ref().is_some_and(|g| !g.cap.is_current()) {
        app.win = None;
        app.gfx = None;
    }
    if app.gfx.is_none() {
        app.gfx = Some(Gfx::new()?);
        tl.mark("gfx");
    }
    let gfx = app.gfx.as_mut().unwrap();
    let (frames, attempts) = gfx.cap.capture_gpu()?;
    tl.mark("capture");
    let frames = gfx.bitmaps(frames)?;
    tl.mark("bitmap");
    let vrect = gfx.cap.virtual_rect();
    if app.win.as_ref().is_some_and(|w| w.rect != vrect) {
        app.win = None;
    }
    if app.win.is_none() {
        app.win = Some(Win::new(gfx, vrect)?);
        tl.mark("window");
    }
    let win = app.win.as_ref().unwrap();
    let session = Session {
        n,
        frames,
        anchor: (0, 0),
        sel: None,
        dragging: false,
    };
    unsafe {
        // 先以 cloak 状态显示并画好第一帧，再取消 cloak：避免闪出旧内容或空白。
        SetWindowPos(
            win.hwnd,
            Some(HWND_TOPMOST),
            vrect.left,
            vrect.top,
            vrect.right - vrect.left,
            vrect.bottom - vrect.top,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        )?;
        let mode = std::env::var("BPS_SHOW_MODE").unwrap_or_default();
        render(gfx, win, &session, if mode == "imm" { 0 } else { 1 })?;
        tl.mark("present");
        if mode == "flush" {
            let _ = DwmFlush();
            tl.mark("preflush");
        }
        cloak(win.hwnd, false);
        tl.mark("uncloak");
        let _ = SetForegroundWindow(win.hwnd);
        let _ = SetFocus(Some(win.hwnd));
        tl.mark("foreground");
        let _ = DwmFlush();
        tl.mark("dwm");
        let fg = GetForegroundWindow() == win.hwnd;
        if app.args.hold_ms > 0 && !app.hold_next {
            SetTimer(
                Some(win.hwnd),
                TIMER_AUTOCLOSE,
                app.args.hold_ms as u32,
                None,
            );
        }
        SetTimer(
            Some(win.hwnd),
            TIMER_SAFETY,
            proto::SAFETY_CLOSE_MS as u32,
            None,
        );
        app.hold_next = false;
        tl.emit(&format!("attempts={attempts} fg={}", fg as u8));
    }
    app.session = Some(session);
    Ok(())
}

fn close(app: &mut App, reason: &str) {
    let Some(s) = app.session.take() else { return };
    if let Some(win) = &app.win {
        unsafe {
            let _ = KillTimer(Some(win.hwnd), TIMER_AUTOCLOSE);
            let _ = KillTimer(Some(win.hwnd), TIMER_SAFETY);
            cloak(win.hwnd, true);
            let _ = ShowWindow(win.hwnd, SW_HIDE);
        }
    }
    drop(s);
    match app.args.strategy {
        Strategy::Prewarm => {}
        Strategy::OnDemand => app.win = None,
        Strategy::Lazy => {
            app.win = None;
            app.gfx = None;
        }
    }
    if let Some(g) = &app.gfx {
        // 让 D3D11 立即回收冻结帧纹理，而不是等到下一次提交。
        let a = &g.cap.adapters[g.render_adapter];
        unsafe {
            a.context.ClearState();
            a.context.Flush();
        }
    }
    emit(&format!("CLOSED n={} reason={reason}", app.count));
}

fn on_cmd(app: &mut App, cmd: usize) -> bool {
    match cmd {
        0 => app.hold_next = true,
        1 => close(app, "cmd"),
        _ => return false,
    }
    true
}

fn lparam_pt(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

fn norm(a: (i32, i32), b: (i32, i32)) -> RECT {
    RECT {
        left: a.0.min(b.0),
        top: a.1.min(b.1),
        right: a.0.max(b.0),
        bottom: a.1.max(b.1),
    }
}

/// 处理遮罩窗口消息；返回 None 表示交给 DefWindowProc。
fn handle(app: &mut App, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let redraw = |app: &mut App| {
        if let (Some(g), Some(w), Some(s)) =
            (app.gfx.as_mut(), app.win.as_ref(), app.session.as_ref())
        {
            let _ = render(g, w, s, 1);
        }
    };
    match msg {
        WM_LBUTTONDOWN => {
            let s = app.session.as_mut()?;
            let p = lparam_pt(lp);
            s.anchor = p;
            s.dragging = true;
            s.sel = Some(norm(p, p));
            unsafe { SetCapture(hwnd) };
            redraw(app);
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            let s = app.session.as_mut()?;
            if s.dragging {
                s.sel = Some(norm(s.anchor, lparam_pt(lp)));
                redraw(app);
            }
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let s = app.session.as_mut()?;
            if s.dragging {
                s.dragging = false;
                s.sel = Some(norm(s.anchor, lparam_pt(lp)));
                unsafe {
                    let _ = ReleaseCapture();
                }
                let r = s.sel.unwrap();
                let o = app
                    .win
                    .as_ref()
                    .map(|w| (w.rect.left, w.rect.top))
                    .unwrap_or((0, 0));
                emit(&format!(
                    "SELECTION n={} x={} y={} w={} h={}",
                    s.n,
                    r.left + o.0,
                    r.top + o.1,
                    r.right - r.left,
                    r.bottom - r.top
                ));
                redraw(app);
            }
            Some(LRESULT(0))
        }
        WM_RBUTTONUP => {
            close(app, "rclick");
            Some(LRESULT(0))
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            close(app, "esc");
            Some(LRESULT(0))
        }
        WM_TIMER => {
            close(
                app,
                if wp.0 == TIMER_AUTOCLOSE {
                    "timeout"
                } else {
                    "safety"
                },
            );
            Some(LRESULT(0))
        }
        // 横跨多块缩放不同的显示器时，系统会按“窗口主要所在的显示器”发 WM_DPICHANGED 并建议
        // 新尺寸；遮罩必须保持物理像素尺寸不变，所以直接忽略。
        WM_DPICHANGED => Some(LRESULT(0)),
        WM_DISPLAYCHANGE => {
            close(app, "displaychange");
            None
        }
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            Some(LRESULT(0))
        }
        WM_ERASEBKGND => Some(LRESULT(1)),
        _ => None,
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // trigger/close 内部调用 SetWindowPos、DestroyWindow 等会同步重入窗口过程，此时 APP
    // 已被借用，直接走默认处理即可。
    let r = APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut a) => a.as_mut().and_then(|app| handle(app, hwnd, msg, wp, lp)),
        Err(_) => None,
    });
    match r {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn main() -> Result<()> {
    unsafe {
        // 正式实现应写进 manifest；原型里用 API 声明，效果相同。
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
    }
    let args = proto::parse_args();
    let t_start = qpc();
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }

    let mut app = App {
        args: args.clone(),
        gfx: None,
        win: None,
        session: None,
        count: 0,
        hold_next: false,
    };
    match args.strategy {
        Strategy::Prewarm => {
            let gfx = Gfx::new()?;
            let win = Win::new(&gfx, gfx.cap.virtual_rect())?;
            app.gfx = Some(gfx);
            app.win = Some(win);
            // 完整预热：真实拍一次冻结帧、画一帧（含文字），把驱动、着色器、字体缓存都热起来。
            let gfx = app.gfx.as_mut().unwrap();
            let (frames, _) = gfx.cap.capture_gpu()?;
            let frames = gfx.bitmaps(frames)?;
            let mut warm = Session {
                n: 0,
                frames,
                anchor: (0, 0),
                sel: None,
                dragging: false,
            };
            render(gfx, app.win.as_ref().unwrap(), &warm, 1)?;
            warm.sel = Some(RECT {
                left: 100,
                top: 100,
                right: 300,
                bottom: 200,
            });
            render(gfx, app.win.as_ref().unwrap(), &warm, 1)?;
        }
        Strategy::OnDemand => app.gfx = Some(Gfx::new()?),
        Strategy::Lazy => {}
    }
    let t_init = qpc();

    unsafe {
        if let Err(e) = RegisterHotKey(None, proto::HOTKEY_ID, proto::HOTKEY_MODS, proto::HOTKEY_VK)
        {
            emit(&format!("ERROR hotkey {e}"));
            std::process::exit(2);
        }
    }
    let main_tid = unsafe { GetCurrentThreadId() };
    if args.harness {
        proto::spawn_stdin_reader(move |cmd| unsafe {
            let code = match cmd {
                Cmd::Hold => 0,
                Cmd::Close => 1,
                Cmd::Quit => 2,
            };
            let _ = PostThreadMessageW(main_tid, WM_APP_CMD, WPARAM(code), LPARAM(0));
        });
    }
    let exit_timer = unsafe { SetTimer(None, 0, (args.exit_after_s * 1000) as u32, None) };

    let outputs = capture::describe_outputs()?;
    emit(&format!(
        "READY pid={} strategy={} init_ms={:.1} outputs={outputs}",
        std::process::id(),
        args.strategy.name(),
        (t_init - t_start) as f64 * 1000.0 / proto::qpf() as f64
    ));
    APP.with(|c| *c.borrow_mut() = Some(app));

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() {
                match msg.message {
                    WM_HOTKEY if msg.wParam.0 == proto::HOTKEY_ID as usize => {
                        let t = qpc();
                        APP.with(|c| {
                            let mut a = c.borrow_mut();
                            if let Err(e) = trigger(a.as_mut().unwrap(), t) {
                                emit(&format!("ERROR trigger {e}"));
                            }
                        });
                        continue;
                    }
                    WM_APP_CMD => {
                        let keep =
                            APP.with(|c| on_cmd(c.borrow_mut().as_mut().unwrap(), msg.wParam.0));
                        if !keep {
                            break;
                        }
                        continue;
                    }
                    WM_TIMER if msg.wParam.0 == exit_timer => break,
                    _ => {}
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnregisterHotKey(None, proto::HOTKEY_ID);
    }
    APP.with(|c| {
        if let Some(mut app) = c.borrow_mut().take() {
            close(&mut app, "exit");
        }
    });
    emit("EXIT");
    Ok(())
}
