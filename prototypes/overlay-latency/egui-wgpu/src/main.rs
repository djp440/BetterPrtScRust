//! 一次性原型代码（ticket #2）：egui（winit + wgpu/DX12）选区遮罩。
//!
//! 常驻进程：热键线程 RegisterHotKey → 收到 WM_HOTKEY 用 EventLoopProxy 唤醒 winit 事件循环
//! → DXGI Desktop Duplication 拍冻结帧（D3D11，读回 CPU）→ 上传成 wgpu 纹理 → egui 绘制
//! 冻结帧 + 半透明遮罩，一个铺满虚拟桌面的 winit 窗口，可拖拽框选。
//! egui 只当绘制层用（pixels_per_point 固定 1.0，坐标即物理像素）。不要复用到正式代码。

#[path = "../../common/capture.rs"]
mod capture;
#[path = "../../common/proto.rs"]
mod proto;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use proto::{Args, Cmd, Strategy, Timeline, emit, qpc};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmFlush, DwmSetWindowAttribute};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, SetFocus, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId, WindowLevel};

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

enum UserEvent {
    Hotkey(i64),
    Cmd(Cmd),
}

/// GPU 资源：截屏用的 D3D11 设备（Capturer）+ wgpu DX12 设备 + egui 渲染器。
struct Gpu {
    cap: capture::Capturer,
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    adapter_name: String,
}

impl Gpu {
    fn new() -> AnyResult<Self> {
        let cap = capture::Capturer::new()?;
        let primary = cap.primary().ok_or("没有主显示器")?.adapter_index;
        let pdesc = cap.adapters[primary].desc;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // 关键：选“驱动主显示器的那块显卡”，否则混合显卡机器上会跨适配器呈现，很慢。
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
        let adapter = match adapters.into_iter().find(|a| {
            let i = a.get_info();
            i.vendor == pdesc.VendorId && i.device == pdesc.DeviceId
        }) {
            Some(a) => a,
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))?,
        };
        let adapter_name = adapter.get_info().name;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("overlay"),
                // 虚拟桌面可能比默认 8192 上限更宽。
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }))?;
        let renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        Ok(Self {
            cap,
            instance,
            device,
            queue,
            renderer,
            adapter_name,
        })
    }
}

struct Win {
    window: Arc<Window>,
    hwnd: HWND,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    rect: RECT,
}

static ORIG_WNDPROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// winit 收到 WM_DPICHANGED 时一定会 SetWindowPos 到系统建议的位置，并把窗口“推”到新 DPI
/// 的显示器上——对铺满虚拟桌面的遮罩来说就是错位。这里子类化窗口，直接吞掉该消息。
extern "system" fn swallow_dpi(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DPICHANGED {
        return LRESULT(0);
    }
    unsafe {
        let orig: WNDPROC =
            std::mem::transmute(ORIG_WNDPROC.load(std::sync::atomic::Ordering::Relaxed));
        CallWindowProcW(orig, hwnd, msg, wp, lp)
    }
}

fn cloak(hwnd: HWND, on: bool) {
    let v = BOOL::from(on);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &v as *const BOOL as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

impl Win {
    fn new(el: &ActiveEventLoop, gpu: &Gpu, rect: RECT) -> AnyResult<Self> {
        let (w, h) = (
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        );
        let attrs = Window::default_attributes()
            .with_title("overlay-egui")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(rect.left, rect.top))
            .with_inner_size(PhysicalSize::new(w, h))
            .with_skip_taskbar(true)
            .with_no_redirection_bitmap(true)
            .with_drag_and_drop(false)
            .with_undecorated_shadow(false);
        let window = Arc::new(el.create_window(attrs)?);
        let hwnd = match window.window_handle()?.as_raw() {
            RawWindowHandle::Win32(h) => HWND(h.hwnd.get() as *mut _),
            _ => unreachable!(),
        };
        cloak(hwnd, true);
        unsafe {
            let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, swallow_dpi as *const () as isize);
            ORIG_WNDPROC.store(prev, std::sync::atomic::Ordering::Relaxed);
        }
        let surface = gpu.instance.create_surface(window.clone())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        Ok(Self {
            window,
            hwnd,
            surface,
            config,
            rect,
        })
    }
}

struct Frozen {
    info: capture::OutputInfo,
    _texture: wgpu::Texture,
    id: egui::TextureId,
}

struct Session {
    n: u32,
    frames: Vec<Frozen>,
    anchor: (i32, i32),
    cursor: (i32, i32),
    sel: Option<RECT>,
    dragging: bool,
}

fn upload(gpu: &mut Gpu, frames: Vec<capture::CpuFrame>) -> Vec<Frozen> {
    frames
        .into_iter()
        .map(|f| {
            let size = wgpu::Extent3d {
                width: f.width,
                height: f.height,
                depth_or_array_layers: 1,
            };
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frozen-frame"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // 与桌面复制的 BGRA 一致，免去 CPU 端换序；非 sRGB，保证像素值原样显示。
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &f.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(f.width * 4),
                    rows_per_image: Some(f.height),
                },
                size,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let id =
                gpu.renderer
                    .register_native_texture(&gpu.device, &view, wgpu::FilterMode::Nearest);
            Frozen {
                info: f.info,
                _texture: texture,
                id,
            }
        })
        .collect()
}

fn rect_of(r: RECT, ox: i32, oy: i32) -> Rect {
    Rect::from_min_max(
        pos2((r.left - ox) as f32, (r.top - oy) as f32),
        pos2((r.right - ox) as f32, (r.bottom - oy) as f32),
    )
}

fn paint(ctx: &egui::Context, s: &Session, vrect: RECT) {
    let (ox, oy) = (vrect.left, vrect.top);
    let full = rect_of(vrect, ox, oy);
    let p = ctx.layer_painter(egui::LayerId::background());
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    for f in &s.frames {
        p.image(f.id, rect_of(f.info.rect, ox, oy), uv, Color32::WHITE);
    }
    let mask = Color32::from_black_alpha(115);
    match s.sel {
        None => {
            p.rect_filled(full, 0.0, mask);
        }
        Some(r) => {
            let sel = rect_of(r, 0, 0);
            let (w, h) = (full.width(), full.height());
            p.rect_filled(
                Rect::from_min_max(pos2(0.0, 0.0), pos2(w, sel.top())),
                0.0,
                mask,
            );
            p.rect_filled(
                Rect::from_min_max(pos2(0.0, sel.bottom()), pos2(w, h)),
                0.0,
                mask,
            );
            p.rect_filled(
                Rect::from_min_max(pos2(0.0, sel.top()), pos2(sel.left(), sel.bottom())),
                0.0,
                mask,
            );
            p.rect_filled(
                Rect::from_min_max(pos2(sel.right(), sel.top()), pos2(w, sel.bottom())),
                0.0,
                mask,
            );
            // 按选区左上角所在显示器的缩放算 UI 尺寸：一个窗口横跨多块缩放不同的显示器时，
            // egui 只有一个 pixels_per_point，必须自己按显示器换算。
            let (vx, vy) = (r.left + ox, r.top + oy);
            let dpi = s
                .frames
                .iter()
                .find(|f| {
                    let o = f.info.rect;
                    vx >= o.left && vx < o.right && vy >= o.top && vy < o.bottom
                })
                .map(|f| f.info.dpi)
                .unwrap_or(96);
            let scale = dpi as f32 / 96.0;
            p.rect_stroke(
                sel,
                0.0,
                Stroke::new(scale.round().max(1.0), Color32::from_rgb(30, 144, 255)),
                StrokeKind::Outside,
            );
            let lh = 22.0 * scale;
            let ty = if sel.top() - lh - 4.0 * scale >= 0.0 {
                sel.top() - lh - 4.0 * scale
            } else {
                sel.top() + 4.0 * scale
            };
            let label = Rect::from_min_size(pos2(sel.left(), ty), vec2(110.0 * scale, lh));
            p.rect_filled(label, 0.0, Color32::from_black_alpha(178));
            p.text(
                label.left_center() + vec2(6.0 * scale, 0.0),
                Align2::LEFT_CENTER,
                format!("{} × {}", r.right - r.left, r.bottom - r.top),
                FontId::proportional(13.0 * scale),
                Color32::WHITE,
            );
        }
    }
    // 哨兵：主显示器 (0,0) 处 8×8 品红方块，绿色分量编码触发序号。
    let sz = proto::SENTINEL_SIZE as f32;
    p.rect_filled(
        Rect::from_min_size(pos2(-ox as f32, -oy as f32), vec2(sz, sz)),
        0.0,
        Color32::from_rgb(255, proto::sentinel_code(s.n), 255),
    );
}

fn render(gpu: &mut Gpu, win: &Win, ctx: &egui::Context, s: &Session) -> AnyResult<()> {
    let (w, h) = (win.config.width, win.config.height);
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
        ..Default::default()
    };
    let out = ctx.run_pass(raw, |ctx| paint(ctx, s, win.rect));
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    for (id, deltas) in &out.textures_delta.set {
        for delta in deltas {
            gpu.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let frame = match win.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
        _ => {
            win.surface.configure(&gpu.device, &win.config);
            match win.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t)
                | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                _ => return Err("拿不到交换链缓冲".into()),
            }
        }
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let sd = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [w, h],
        pixels_per_point: 1.0,
    };
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let cbs = gpu
        .renderer
        .update_buffers(&gpu.device, &gpu.queue, &mut enc, &prims, &sd);
    {
        let rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let mut rp = rp.forget_lifetime();
        gpu.renderer.render(&mut rp, &prims, &sd);
    }
    gpu.queue
        .submit(cbs.into_iter().chain(std::iter::once(enc.finish())));
    win.window.pre_present_notify();
    gpu.queue.present(frame);
    for id in &out.textures_delta.free {
        gpu.renderer.free_texture(id);
    }
    Ok(())
}

struct App {
    args: Args,
    ctx: egui::Context,
    gpu: Option<Gpu>,
    win: Option<Win>,
    session: Option<Session>,
    count: u32,
    hold_next: bool,
    close_at: Option<Instant>,
    safety_at: Option<Instant>,
    exit_at: Instant,
    started: bool,
    t_start: i64,
    hotkey_tid: u32,
}

impl App {
    fn trigger(&mut self, el: &ActiveEventLoop, t_hotkey: i64) -> AnyResult<()> {
        if self.session.is_some() {
            return Ok(());
        }
        self.count += 1;
        let n = self.count;
        let mut tl = Timeline::new(n, t_hotkey);
        tl.mark("wake");
        if self.gpu.as_ref().is_some_and(|g| !g.cap.is_current()) {
            self.win = None;
            self.gpu = None;
        }
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::new()?);
            tl.mark("gfx");
        }
        let gpu = self.gpu.as_mut().unwrap();
        let (frames, attempts) = gpu.cap.capture_cpu()?;
        tl.mark("capture");
        let frames = upload(gpu, frames);
        tl.mark("upload");
        let vrect = gpu.cap.virtual_rect();
        if self.win.as_ref().is_some_and(|w| w.rect != vrect) {
            self.win = None;
        }
        if self.win.is_none() {
            self.win = Some(Win::new(el, gpu, vrect)?);
            tl.mark("window");
        }
        let win = self.win.as_ref().unwrap();
        let session = Session {
            n,
            frames,
            anchor: (0, 0),
            cursor: (0, 0),
            sel: None,
            dragging: false,
        };
        unsafe {
            // 与原生原型相同的显示顺序：cloak 状态下显示 → 画第一帧 → 取消 cloak → 抢前台。
            // 不用 winit 的 set_visible/focus_window：前者会同步激活窗口（慢），后者会注入 Alt 键。
            SetWindowPos(
                win.hwnd,
                Some(HWND_TOPMOST),
                vrect.left,
                vrect.top,
                vrect.right - vrect.left,
                vrect.bottom - vrect.top,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )?;
        }
        render(gpu, win, &self.ctx, &session)?;
        tl.mark("present");
        if std::env::var("BPS_SHOW_MODE").is_ok_and(|m| m == "flush") {
            // 实验：等 GPU 做完并过一次 DWM 合成再取消 cloak，看能否消除旧帧闪现。
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            unsafe {
                let _ = DwmFlush();
            }
            tl.mark("preflush");
        }
        cloak(win.hwnd, false);
        tl.mark("uncloak");
        unsafe {
            let _ = SetForegroundWindow(win.hwnd);
            let _ = SetFocus(Some(win.hwnd));
        }
        tl.mark("foreground");
        unsafe {
            let _ = DwmFlush();
        }
        tl.mark("dwm");
        let fg = unsafe { GetForegroundWindow() } == win.hwnd;
        let now = Instant::now();
        if self.args.hold_ms > 0 && !self.hold_next {
            self.close_at = Some(now + Duration::from_millis(self.args.hold_ms));
        }
        self.safety_at = Some(now + Duration::from_millis(proto::SAFETY_CLOSE_MS));
        self.hold_next = false;
        tl.emit(&format!("attempts={attempts} fg={}", fg as u8));
        self.session = Some(session);
        Ok(())
    }

    fn close(&mut self, reason: &str) {
        let Some(s) = self.session.take() else { return };
        self.close_at = None;
        self.safety_at = None;
        if let Some(win) = &self.win {
            cloak(win.hwnd, true);
            unsafe {
                let _ = ShowWindow(win.hwnd, SW_HIDE);
            }
        }
        if let Some(gpu) = self.gpu.as_mut() {
            for f in &s.frames {
                gpu.renderer.free_texture(&f.id);
            }
        }
        drop(s);
        match self.args.strategy {
            Strategy::Prewarm => {}
            Strategy::OnDemand => self.win = None,
            Strategy::Lazy => {
                self.win = None;
                self.gpu = None;
            }
        }
        if let Some(gpu) = &self.gpu {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        emit(&format!("CLOSED n={} reason={reason}", self.count));
    }

    fn redraw(&mut self) {
        if let (Some(g), Some(w), Some(s)) =
            (self.gpu.as_mut(), self.win.as_ref(), self.session.as_ref())
        {
            let _ = render(g, w, &self.ctx, s);
        }
    }

    fn startup(&mut self, el: &ActiveEventLoop) -> AnyResult<()> {
        match self.args.strategy {
            Strategy::Prewarm => {
                let gpu = Gpu::new()?;
                let win = Win::new(el, &gpu, gpu.cap.virtual_rect())?;
                self.gpu = Some(gpu);
                self.win = Some(win);
                // 完整预热：真实拍一次、上传、画两帧（含文字），把驱动、管线、字体图集都热起来。
                let gpu = self.gpu.as_mut().unwrap();
                let (frames, _) = gpu.cap.capture_cpu()?;
                let frames = upload(gpu, frames);
                let mut warm = Session {
                    n: 0,
                    frames,
                    anchor: (0, 0),
                    cursor: (0, 0),
                    sel: None,
                    dragging: false,
                };
                let win = self.win.as_ref().unwrap();
                render(gpu, win, &self.ctx, &warm)?;
                warm.sel = Some(RECT {
                    left: 100,
                    top: 100,
                    right: 300,
                    bottom: 200,
                });
                render(gpu, win, &self.ctx, &warm)?;
                for f in &warm.frames {
                    gpu.renderer.free_texture(&f.id);
                }
                drop(warm);
                let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            }
            Strategy::OnDemand => self.gpu = Some(Gpu::new()?),
            Strategy::Lazy => {}
        }
        Ok(())
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        if let Err(e) = self.startup(el) {
            emit(&format!("ERROR startup {e}"));
            el.exit();
            return;
        }
        let t_init = qpc();
        let outputs = capture::describe_outputs().unwrap_or_default();
        let adapter = self
            .gpu
            .as_ref()
            .map(|g| g.adapter_name.replace(' ', "_"))
            .unwrap_or_else(|| "-".into());
        emit(&format!(
            "READY pid={} strategy={} init_ms={:.1} wgpu_adapter={adapter} outputs={outputs}",
            std::process::id(),
            self.args.strategy.name(),
            (t_init - self.t_start) as f64 * 1000.0 / proto::qpf() as f64
        ));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Hotkey(t) => {
                if let Err(e) = self.trigger(el, t) {
                    emit(&format!("ERROR trigger {e}"));
                }
            }
            UserEvent::Cmd(Cmd::Hold) => self.hold_next = true,
            UserEvent::Cmd(Cmd::Close) => self.close("cmd"),
            UserEvent::Cmd(Cmd::Quit) => el.exit(),
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CursorMoved { position, .. } => {
                let Some(s) = self.session.as_mut() else {
                    return;
                };
                s.cursor = (position.x as i32, position.y as i32);
                if s.dragging {
                    s.sel = Some(norm(s.anchor, s.cursor));
                    self.redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let Some(s) = self.session.as_mut() else {
                    return;
                };
                match state {
                    ElementState::Pressed => {
                        s.anchor = s.cursor;
                        s.dragging = true;
                        s.sel = Some(norm(s.cursor, s.cursor));
                    }
                    ElementState::Released if s.dragging => {
                        s.dragging = false;
                        let r = norm(s.anchor, s.cursor);
                        s.sel = Some(r);
                        let o = self
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
                    }
                    _ => {}
                }
                self.redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Right,
                ..
            } => self.close("rclick"),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                self.close("esc")
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if self.close_at.is_some_and(|t| now >= t) {
            self.close("timeout");
        }
        if self.safety_at.is_some_and(|t| now >= t) {
            self.close("safety");
        }
        if now >= self.exit_at {
            el.exit();
            return;
        }
        let next = [self.close_at, self.safety_at, Some(self.exit_at)]
            .into_iter()
            .flatten()
            .min()
            .unwrap();
        el.set_control_flow(ControlFlow::WaitUntil(next));
    }
}

fn norm(a: (i32, i32), b: (i32, i32)) -> RECT {
    RECT {
        left: a.0.min(b.0),
        top: a.1.min(b.1),
        right: a.0.max(b.0),
        bottom: a.1.max(b.1),
    }
}

/// 热键线程：winit 的消息循环拿不到线程消息（hwnd 为空的 WM_HOTKEY），所以单独开线程注册热键，
/// 收到后用 EventLoopProxy 唤醒事件循环。
fn spawn_hotkey_thread(proxy: EventLoopProxy<UserEvent>) -> (u32, std::thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let ok = RegisterHotKey(None, proto::HOTKEY_ID, proto::HOTKEY_MODS, proto::HOTKEY_VK);
        let _ = tx.send((
            GetCurrentThreadId(),
            ok.as_ref().err().map(|e| e.to_string()),
        ));
        if ok.is_err() {
            return;
        }
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                let t = qpc();
                let _ = proxy.send_event(UserEvent::Hotkey(t));
            }
        }
        let _ = UnregisterHotKey(None, proto::HOTKEY_ID);
    });
    let (tid, err) = rx.recv().unwrap();
    if let Some(e) = err {
        emit(&format!("ERROR hotkey {e}"));
        std::process::exit(2);
    }
    (tid, h)
}

fn main() -> AnyResult<()> {
    unsafe {
        // winit 建事件循环时也会声明 Per-Monitor V2；这里提前声明，保证截屏代码拿到物理像素。
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args = proto::parse_args();
    let t_start = qpc();
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let ctx = egui::Context::default();
    // 不要羽化（抗锯齿边缘）：遮罩四块矩形拼接处和哨兵都需要整像素输出。
    ctx.tessellation_options_mut(|o| o.feathering = false);
    let (hotkey_tid, hotkey_thread) = spawn_hotkey_thread(proxy.clone());
    if args.harness {
        let p = proxy.clone();
        proto::spawn_stdin_reader(move |c| {
            let _ = p.send_event(UserEvent::Cmd(c));
        });
    }
    let mut app = App {
        exit_at: Instant::now() + Duration::from_secs(args.exit_after_s),
        args,
        ctx,
        gpu: None,
        win: None,
        session: None,
        count: 0,
        hold_next: false,
        close_at: None,
        safety_at: None,
        started: false,
        t_start,
        hotkey_tid,
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app)?;
    app.close("exit");
    unsafe {
        let _ = PostThreadMessageW(app.hotkey_tid, WM_QUIT, WPARAM(0), LPARAM(0));
    }
    let _ = hotkey_thread.join();
    drop(app);
    emit("EXIT");
    Ok(())
}
