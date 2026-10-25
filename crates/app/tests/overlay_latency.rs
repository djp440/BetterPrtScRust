//! 选区遮罩端到端延迟（沿用 #2 harness 的方法），真机测量，默认忽略。
//!
//! 运行（务必 release）：
//! `cargo test --release -p bps-app --test overlay_latency -- --ignored --nocapture`
//! 可用环境变量 `BPS_LATENCY_COLD_RUNS`（默认 5）、`BPS_LATENCY_TRIGGERS`（每次冷启动后的
//! 后续触发次数，默认 10）调整次数。
//!
//! 过程：冷启动 release 版 `BetterPrtSc.exe`（常驻实例），空闲 2 s 后记录私有工作集。
//! 常驻实例的 `APPDATA` 指向临时目录，那里预先写好只有一条“区域 + 复制”绑定的配置文件
//! （测的只是遮罩出现的延迟，不需要默认 PrtSc 的标注），不碰真实的 %APPDATA%。
//! 在主显示器左上角放一个置顶小色块窗口，每次触发前换一种颜色；
//! 向常驻实例投递这条绑定的 `WM_HOTKEY`（`t0` = 投递前的 QPC），
//! 另开一路 Desktop Duplication 盯着色块：第一次看到“被遮罩压暗的本次色块”的桌面帧，
//! 其 `LastPresentTime` 就是遮罩真正上屏的时刻，延迟 = `LastPresentTime − t0`。
//! 遮罩显示 250 ms 后向遮罩窗口投递 Esc 关闭，间隔 250–450 ms 再触发下一次。
//!
//! 与 #2 的差别：#2 用 SendInput 按下真实热键，这里直接投递 `WM_HOTKEY`（PrtSc 可能被
//! “截图工具”占用，且不想向桌面注入按键），不含“按键 → WM_HOTKEY”这一段（通常 < 1 ms）。
//! 运行期间屏幕会反复闪现遮罩，请勿操作电脑。结果写入仓库 `logs/<时间>.log`。
#![cfg(windows)]

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bps_platform_windows::dpi;
use windows::Win32::Foundation::{COLORREF, HANDLE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, InvalidateRect,
    PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW,
    GetWindowThreadProcessId, IsWindowVisible, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
    RegisterClassExW, SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage, WM_CLOSE, WM_HOTKEY,
    WM_KEYDOWN, WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Interface, w};

/// 测量用的配置：唯一一条绑定是“区域 + 复制”，热键标识为 1。
const CONFIG: &str = "[[bindings]]
hotkey = \"Ctrl+Alt+Shift+F8\"
mode = \"region\"
after = \"copy_to_clipboard\"
";
const REGION_HOTKEY_ID: usize = 1;
const VK_ESCAPE: usize = 0x1B;
/// 色块边长（物理像素）。
const PATCH: i32 = 32;
/// 每次触发前色块换的绿色分量；相邻两次差得足够大，能区分“上一次遮罩的残影”。
const CODES: [u8; 7] = [60, 200, 110, 230, 40, 160, 90];

static PATCH_COLOR: AtomicU32 = AtomicU32::new(0x00FF_00FF);

fn qpc() -> i64 {
    let mut t = 0;
    unsafe { QueryPerformanceCounter(&mut t) }.unwrap();
    t
}

fn qpc_ms(ticks: i64) -> f64 {
    let mut f = 0;
    unsafe { QueryPerformanceFrequency(&mut f) }.unwrap();
    ticks as f64 * 1000.0 / f as f64
}

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ------------------------------------------------------------------ 观察者

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn hue(self) -> f32 {
        f32::from(self.1) / f32::from(self.0.max(1))
    }
}

#[derive(Default)]
struct ProbeState {
    /// 最近一帧里色块中心的颜色。
    latest: Option<Rgb>,
    armed: Option<Armed>,
}

struct Armed {
    baseline: Rgb,
    found: Option<i64>,
    /// 先看到的“压暗了但颜色不是本次色块”的帧数（上一次遮罩的残影）。
    stale: u32,
}

/// 色块被遮罩压暗：两个饱和分量明显变暗但没有消失。
fn dimmed(baseline: Rgb, now: Rgb) -> bool {
    let darker = |b: u8, n: u8| {
        let (b, n) = (f32::from(b), f32::from(n));
        n < 0.85 * b && n > 0.25 * b
    };
    darker(baseline.0, now.0) && darker(baseline.2, now.2)
}

struct Watcher {
    shared: Arc<(Mutex<ProbeState>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    fn start() -> Self {
        let shared = Arc::new((Mutex::new(ProbeState::default()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::spawn(move || {
                if let Err(e) = watch(&shared, &stop, &ready_tx) {
                    eprintln!("观察者出错：{e}");
                    let _ = ready_tx.send(false);
                }
            })
        };
        assert!(ready_rx.recv().unwrap_or(false), "观察者启动失败");
        Self {
            shared,
            stop,
            thread: Some(thread),
        }
    }

    fn latest(&self) -> Option<Rgb> {
        self.shared.0.lock().unwrap().latest
    }

    fn arm(&self, baseline: Rgb) {
        self.shared.0.lock().unwrap().armed = Some(Armed {
            baseline,
            found: None,
            stale: 0,
        });
    }

    /// 遮罩已经上屏时返回该帧的 LastPresentTime。
    fn found(&self) -> Option<i64> {
        self.shared.0.lock().unwrap().armed.as_ref()?.found
    }

    /// 结束这一次观察，返回之前看到的残影帧数。
    fn disarm(&self) -> u32 {
        self.shared
            .0
            .lock()
            .unwrap()
            .armed
            .take()
            .map_or(0, |a| a.stale)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 主显示器（桌面坐标原点所在的输出）及其显卡。
fn primary_output() -> windows::core::Result<(IDXGIAdapter1, IDXGIOutput)> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        for a in 0.. {
            let adapter = factory.EnumAdapters1(a)?;
            for o in 0.. {
                let Ok(output) = adapter.EnumOutputs(o) else {
                    break;
                };
                let desc = output.GetDesc()?;
                let r = desc.DesktopCoordinates;
                if desc.AttachedToDesktop.as_bool() && r.left == 0 && r.top == 0 {
                    assert!(
                        desc.Rotation == DXGI_MODE_ROTATION_IDENTITY
                            || desc.Rotation == DXGI_MODE_ROTATION_UNSPECIFIED,
                        "主显示器旋转了，色块位置不对"
                    );
                    return Ok((adapter, output));
                }
            }
        }
        unreachable!()
    }
}

fn duplicate(
    output: &IDXGIOutput,
    device: &ID3D11Device,
) -> windows::core::Result<IDXGIOutputDuplication> {
    unsafe {
        match output.cast::<IDXGIOutput5>() {
            Ok(o5) => o5.DuplicateOutput1(device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
            Err(_) => output.cast::<IDXGIOutput1>()?.DuplicateOutput(device),
        }
    }
}

fn watch(
    shared: &(Mutex<ProbeState>, Condvar),
    stop: &AtomicBool,
    ready: &mpsc::Sender<bool>,
) -> windows::core::Result<()> {
    let (adapter, output) = primary_output()?;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let (device, context) = (device.unwrap(), context.unwrap());
    let mut probe: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(
            &D3D11_TEXTURE2D_DESC {
                Width: 1,
                Height: 1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            },
            None,
            Some(&mut probe),
        )?;
    }
    let probe = probe.unwrap();
    let mut duplication = duplicate(&output, &device)?;
    let _ = ready.send(true);
    let center = (PATCH / 2) as u32;
    while !stop.load(Ordering::SeqCst) {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        match unsafe { duplication.AcquireNextFrame(16, &mut info, &mut resource) } {
            Ok(()) => {
                if info.LastPresentTime != 0 {
                    let texture: ID3D11Texture2D = resource.unwrap().cast()?;
                    let rgb = unsafe {
                        let region = D3D11_BOX {
                            left: center,
                            top: center,
                            front: 0,
                            right: center + 1,
                            bottom: center + 1,
                            back: 1,
                        };
                        context.CopySubresourceRegion(
                            &probe,
                            0,
                            0,
                            0,
                            0,
                            &texture,
                            0,
                            Some(&region),
                        );
                        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                        context.Map(&probe, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                        let p = mapped.pData as *const u8;
                        let rgb = Rgb(*p.add(2), *p.add(1), *p);
                        context.Unmap(&probe, 0);
                        rgb
                    };
                    let (m, cv) = shared;
                    let mut s = m.lock().unwrap();
                    s.latest = Some(rgb);
                    match s.armed.as_mut() {
                        Some(armed) if armed.found.is_none() && dimmed(armed.baseline, rgb) => {
                            if (rgb.hue() - armed.baseline.hue()).abs() < 0.1 {
                                armed.found = Some(info.LastPresentTime);
                            } else {
                                armed.stale += 1;
                            }
                        }
                        _ => {}
                    }
                    cv.notify_all();
                }
                unsafe { duplication.ReleaseFrame() }?;
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(duplication);
                std::thread::sleep(Duration::from_millis(50));
                duplication = duplicate(&output, &device)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ 色块窗口

extern "system" fn patch_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_PAINT {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let brush = CreateSolidBrush(COLORREF(PATCH_COLOR.load(Ordering::SeqCst)));
            let rect = RECT {
                left: 0,
                top: 0,
                right: PATCH,
                bottom: PATCH,
            };
            FillRect(hdc, &rect, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
            let _ = EndPaint(hwnd, &ps);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

struct Patch(HWND);

impl Patch {
    fn show() -> Self {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let class = w!("BetterPrtSc.LatencyPatch");
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(patch_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!(""),
                WS_POPUP,
                0,
                0,
                PATCH,
                PATCH,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(hwnd);
            Self(hwnd)
        }
    }

    /// 换成品红色系里绿色分量为 `code` 的颜色。
    fn paint(&self, code: u8) {
        PATCH_COLOR.store(0x00FF_00FF | (u32::from(code) << 8), Ordering::SeqCst);
        unsafe {
            let _ = InvalidateRect(Some(self.0), None, false);
            let _ = UpdateWindow(self.0);
        }
    }
}

impl Drop for Patch {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// 处理本线程的窗口消息，直到 `done` 成立或超时。
fn pump_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pump_for(duration: Duration) {
    pump_until(duration, || false);
}

// ------------------------------------------------------------------ 常驻实例

/// 进程 `pid` 的、类名为 `class` 的顶层窗口。
fn window_of(pid: u32, class: windows::core::PCWSTR) -> Option<HWND> {
    let mut after = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, class, None) }.ok()?;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == pid {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
}

fn any_window(class: windows::core::PCWSTR) -> bool {
    unsafe { FindWindowExW(None, None, class, None) }.is_ok()
}

struct Resident {
    child: Child,
    lines: Receiver<String>,
    output: Vec<String>,
    resident_window: HWND,
    overlay_window: HWND,
}

impl Resident {
    fn start() -> Self {
        let app_data = std::env::temp_dir().join("bps-overlay-latency");
        let config_dir = app_data.join("BetterPrtSc");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("config.toml"), CONFIG).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_BetterPrtSc"))
            .env("APPDATA", &app_data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("启动 BetterPrtSc.exe 失败");
        let stderr = child.stderr.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut resident = Self {
            child,
            lines,
            output: Vec::new(),
            resident_window: HWND::default(),
            overlay_window: HWND::default(),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !resident.output.iter().any(|l| l.contains("常驻实例已启动")) {
            let left = deadline.saturating_duration_since(Instant::now());
            match resident.lines.recv_timeout(left) {
                Ok(line) => resident.output.push(line),
                Err(_) => panic!("常驻实例没有启动：{:?}", resident.output),
            }
        }
        let pid = resident.child.id();
        resident.resident_window =
            window_of(pid, w!("BetterPrtSc.ResidentWindow")).expect("找不到常驻实例窗口");
        resident.overlay_window =
            window_of(pid, w!("BetterPrtSc.SelectionOverlay")).expect("找不到预热的遮罩窗口");
        resident
    }

    /// 私有工作集（任务管理器“内存”列），MB。
    fn private_working_set_mb(&self) -> f64 {
        let mut m = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                HANDLE(self.child.as_raw_handle()),
                (&raw mut m).cast::<PROCESS_MEMORY_COUNTERS>(),
                m.cb,
            )
        }
        .unwrap();
        m.PrivateWorkingSetSize as f64 / (1024.0 * 1024.0)
    }

    fn post(&self, hwnd: HWND, msg: u32, wparam: usize) {
        unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(0)) }.unwrap();
    }

    fn overlay_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.overlay_window) }.as_bool()
    }

    fn quit(mut self) -> Vec<String> {
        self.post(self.resident_window, WM_CLOSE, 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                self.output.push("!! 5 秒内没有退出，已强制结束".to_owned());
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.wait();
        // 读完剩下的输出（进程退出后管道关闭，读线程随之结束）。
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.output.push(line);
        }
        self.output
    }
}

// ------------------------------------------------------------------ 统计与日志

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn summary(name: &str, values: &[f64]) -> String {
    if values.is_empty() {
        return format!("{name}：无数据");
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    format!(
        "{name}：n={}，中位数 {:.1} ms，P95 {:.1} ms，最小 {:.1} ms，最大 {:.1} ms",
        v.len(),
        percentile(&v, 0.5),
        percentile(&v, 0.95),
        v[0],
        v[v.len() - 1]
    )
}

fn open_log() -> (PathBuf, File) {
    let t = unsafe { GetLocalTime() };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}.log",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    ));
    let file = File::create(&path).unwrap();
    (path, file)
}

/// 简单的伪随机间隔（250–450 ms），避免和刷新周期同相。
fn gap(seed: &mut u64) -> Duration {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    Duration::from_millis(250 + (*seed >> 33) % 200)
}

#[test]
#[ignore = "真机测量：反复启动常驻实例并闪现选区遮罩，约 1 分钟"]
fn hotkey_to_overlay_latency() {
    if cfg!(debug_assertions) {
        panic!("请用 --release 运行：debug 构建的冻结帧转换慢一个数量级");
    }
    assert!(
        !any_window(w!("BetterPrtSc.ResidentWindow")),
        "已有 BetterPrtSc 常驻实例在运行，先退出它再测"
    );
    assert!(dpi::ensure_per_monitor_v2());
    let cold_runs = env_or("BPS_LATENCY_COLD_RUNS", 5);
    let triggers = env_or("BPS_LATENCY_TRIGGERS", 10);

    let (log_path, mut log) = open_log();
    let mut say = |line: String| {
        eprintln!("{line}");
        writeln!(log, "{line}").unwrap();
    };
    say(format!(
        "# 选区遮罩延迟：冷启动 {cold_runs} 次 × (1 次首次 + {triggers} 次后续)"
    ));

    let watcher = Watcher::start();
    let patch = Patch::show();
    let mut seed = qpc() as u64;
    let mut code_index = 0;
    let (mut first, mut later, mut idle_mb, mut after_mb) = (vec![], vec![], vec![], vec![]);
    let mut stale_total = 0;
    let mut outputs = Vec::new();

    for run in 1..=cold_runs {
        let resident = Resident::start();
        pump_for(Duration::from_secs(2));
        let idle = resident.private_working_set_mb();
        idle_mb.push(idle);
        say(format!("## 第 {run} 次冷启动：空闲私有工作集 {idle:.1} MB"));

        for trigger in 0..=triggers {
            let code = CODES[code_index % CODES.len()];
            code_index += 1;
            patch.paint(code);
            let want = f32::from(code) / 255.0;
            let mut baseline = None;
            let shown = pump_until(Duration::from_secs(2), || {
                baseline = watcher
                    .latest()
                    .filter(|c| c.0 >= 200 && c.2 >= 200 && (c.hue() - want).abs() < 0.06);
                baseline.is_some()
            });
            assert!(shown, "观察者没看到色块：{:?}", watcher.latest());
            watcher.arm(baseline.unwrap());

            let t0 = qpc();
            resident.post(resident.resident_window, WM_HOTKEY, REGION_HOTKEY_ID);
            // 时刻取自观察者记下的 LastPresentTime，这里轮询的快慢不影响结果。
            pump_until(Duration::from_secs(3), || watcher.found().is_some());
            let found = watcher.found();
            let stale = watcher.disarm();
            stale_total += stale;
            let latency = found.map(|t| qpc_ms(t - t0));
            say(format!(
                "- {} 触发 {trigger}：{}{}",
                if trigger == 0 { "首次" } else { "后续" },
                latency.map_or("未检测到遮罩".to_owned(), |l| format!("{l:.1} ms")),
                if stale > 0 {
                    format!("（先看到 {stale} 帧残影）")
                } else {
                    String::new()
                }
            ));
            match (latency, trigger) {
                (Some(l), 0) => first.push(l),
                (Some(l), _) => later.push(l),
                _ => {}
            }

            pump_until(Duration::from_secs(2), || resident.overlay_visible());
            pump_for(Duration::from_millis(250));
            resident.post(resident.overlay_window, WM_KEYDOWN, VK_ESCAPE);
            let closed = pump_until(Duration::from_secs(2), || !resident.overlay_visible());
            assert!(closed, "Esc 后遮罩没有关闭");
            pump_for(gap(&mut seed));
        }
        let after = resident.private_working_set_mb();
        after_mb.push(after);
        say(format!("触发后私有工作集 {after:.1} MB"));
        outputs.push(resident.quit());
        pump_for(Duration::from_millis(500));
    }
    drop(patch);
    drop(watcher);
    let _ = fs::remove_dir_all(std::env::temp_dir().join("bps-overlay-latency"));

    say("## 汇总".to_owned());
    say(summary("冷启动后首次（热键 → 遮罩上屏）", &first));
    say(summary("后续", &later));
    say(summary("空闲私有工作集（MB）", &idle_mb).replace(" ms", ""));
    say(summary("触发后私有工作集（MB）", &after_mb).replace(" ms", ""));
    say(format!("残影帧（先闪出上一次遮罩画面）：{stale_total}"));
    for (i, output) in outputs.iter().enumerate() {
        say(format!("## 第 {} 次常驻实例输出", i + 1));
        for line in output {
            say(format!("    {line}"));
        }
    }
    eprintln!("日志：{}", log_path.display());
    let total = cold_runs * (triggers + 1);
    assert_eq!(first.len() + later.len(), total, "有触发没检测到遮罩");
}
