//! 一次性原型代码（ticket #2）：选区遮罩延迟测量工具。
//!
//! 对每个“原型 × 策略”：
//! 1. 冷启动原型进程（相当于常驻实例刚启动），等 READY，空闲 2 s 后记录内存；
//! 2. 用 SendInput 注入热键，`t0` = 注入前的 QPC；
//! 3. 本进程另开一路 DXGI Desktop Duplication 盯着主显示器左上角，原型画的哨兵方块
//!    （绿色分量编码触发序号）第一次出现在桌面帧里时，取该帧的 LastPresentTime 作为
//!    “遮罩真正上屏”的时刻 → 端到端延迟 = LastPresentTime − t0；
//! 3. 第 1 次触发记为“冷启动后首次”，其余记为“后续”；
//! 4. 另做一次拖拽验证：注入鼠标拖拽，核对原型报告的选区，并截下遮罩画面存 PNG。
//!
//! 安全：注入鼠标前确认光标下是原型窗口；注入 Esc 前确认前台是原型窗口；
//! 原型进程最后一定被 quit / kill，热键随进程退出由系统回收。

#[path = "../../common/capture.rs"]
mod capture;
#[path = "../../common/proto.rs"]
mod proto;

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use proto::{qpc, qpf};
use windows::Win32::Foundation::{HANDLE, POINT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Interface;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ms(ticks: i64) -> f64 {
    ticks as f64 * 1000.0 / qpf() as f64
}

// ---------------------------------------------------------------- 观察者

#[derive(Default)]
struct WatchState {
    expect: Option<u8>,
    found: Option<i64>,
    stale: u32,
    trace: Vec<(i64, u8, u8, u8)>,
    snapshot_req: bool,
    snapshot: Option<(u32, u32, Vec<u8>)>,
}

struct Watcher {
    shared: Arc<(Mutex<WatchState>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    fn start() -> Self {
        let shared = Arc::new((Mutex::new(WatchState::default()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, st2) = (shared.clone(), stop.clone());
        let (ready_tx, ready_rx) = channel();
        let thread = std::thread::spawn(move || {
            if let Err(e) = watch_loop(&s2, &st2, &ready_tx) {
                eprintln!("watcher error: {e}");
                let _ = ready_tx.send(false);
            }
        });
        assert!(ready_rx.recv().unwrap_or(false), "观察者启动失败");
        Self {
            shared,
            stop,
            thread: Some(thread),
        }
    }

    fn arm(&self, code: u8) {
        let mut s = self.shared.0.lock().unwrap();
        s.expect = Some(code);
        s.found = None;
        s.stale = 0;
        s.trace.clear();
    }

    /// 等哨兵出现，返回 (LastPresentTime, 之前看到的旧哨兵帧数)。
    fn wait_found(&self, timeout: Duration) -> (Option<i64>, u32) {
        let (m, cv) = &*self.shared;
        let (s, _) = cv
            .wait_timeout_while(m.lock().unwrap(), timeout, |s| s.found.is_none())
            .unwrap();
        (s.found, s.stale)
    }

    fn take_trace(&self) -> Vec<(i64, u8, u8, u8)> {
        std::mem::take(&mut self.shared.0.lock().unwrap().trace)
    }

    fn disarm(&self) {
        self.shared.0.lock().unwrap().expect = None;
    }

    fn snapshot(&self) -> Option<(u32, u32, Vec<u8>)> {
        let (m, cv) = &*self.shared;
        {
            let mut s = m.lock().unwrap();
            s.snapshot = None;
            s.snapshot_req = true;
        }
        let (mut s, _) = cv
            .wait_timeout_while(m.lock().unwrap(), Duration::from_secs(3), |s| {
                s.snapshot.is_none()
            })
            .unwrap();
        s.snapshot.take()
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

fn watch_loop(
    shared: &Arc<(Mutex<WatchState>, Condvar)>,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::Sender<bool>,
) -> windows::core::Result<()> {
    let cap = capture::Capturer::new()?;
    let (actx, output, info) = cap
        .adapters
        .iter()
        .flat_map(|a| a.outputs.iter().map(move |(o, i)| (a, o, i)))
        .find(|(_, _, i)| i.primary)
        .expect("没有主显示器");
    let (dev, ctx) = (&actx.device, &actx.context);
    let make_dup = || unsafe {
        match output.cast::<IDXGIOutput5>() {
            Ok(o5) => o5.DuplicateOutput1(dev, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
            Err(_) => output.DuplicateOutput(dev),
        }
    };
    let mut dup = make_dup()?;
    let (w, h) = (info.width() as u32, info.height() as u32);
    let tex_desc = |usage, bind: u32, cpu: u32, tw, th| D3D11_TEXTURE2D_DESC {
        Width: tw,
        Height: th,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let sz = proto::SENTINEL_SIZE as u32;
    let mut probe = None;
    let mut latest = None;
    let mut full = None;
    unsafe {
        let read = D3D11_CPU_ACCESS_READ.0 as u32;
        dev.CreateTexture2D(
            &tex_desc(D3D11_USAGE_STAGING, 0, read, sz, sz),
            None,
            Some(&mut probe),
        )?;
        dev.CreateTexture2D(
            &tex_desc(D3D11_USAGE_DEFAULT, 0, 0, w, h),
            None,
            Some(&mut latest),
        )?;
        dev.CreateTexture2D(
            &tex_desc(D3D11_USAGE_STAGING, 0, read, w, h),
            None,
            Some(&mut full),
        )?;
    }
    let (probe, latest, full) = (probe.unwrap(), latest.unwrap(), full.unwrap());
    // 哨兵在主显示器 (0,0)，即主显示器纹理的左上角。
    let (px, py) = (0u32, 0u32);
    let _ = ready.send(true);
    let mut have_latest = false;
    while !stop.load(Ordering::SeqCst) {
        let mut fi = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe { dup.AcquireNextFrame(16, &mut fi, &mut res) } {
            Ok(()) => unsafe {
                if fi.LastPresentTime != 0 {
                    let tex: ID3D11Texture2D = res.unwrap().cast()?;
                    let bx = D3D11_BOX {
                        left: px,
                        top: py,
                        front: 0,
                        right: px + sz,
                        bottom: py + sz,
                        back: 1,
                    };
                    ctx.CopySubresourceRegion(&probe, 0, 0, 0, 0, &tex, 0, Some(&bx));
                    ctx.CopyResource(&latest, &tex);
                    have_latest = true;
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    ctx.Map(&probe, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
                    let c = sz as usize / 2;
                    let p = (m.pData as *const u8).add(c * m.RowPitch as usize + c * 4);
                    let (b, g, r) = (*p, *p.add(1), *p.add(2));
                    ctx.Unmap(&probe, 0);
                    let (mtx, cv) = &**shared;
                    let mut s = mtx.lock().unwrap();
                    if let Some(code) = s.expect {
                        if s.found.is_none() {
                            s.trace.push((fi.LastPresentTime, r, g, b));
                        }
                        // 哨兵：R、B 饱和，G 落在编码区间 [28, 228)。
                        if r >= 240 && b >= 240 && (20..=235).contains(&g) {
                            if (g as i32 - code as i32).abs() <= 6 {
                                if s.found.is_none() {
                                    s.found = Some(fi.LastPresentTime);
                                    cv.notify_all();
                                }
                            } else if s.found.is_none() {
                                s.stale += 1;
                            }
                        }
                    }
                }
                dup.ReleaseFrame()?;
            },
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(dup);
                std::thread::sleep(Duration::from_millis(50));
                dup = make_dup()?;
                continue;
            }
            Err(e) => return Err(e),
        }
        let want = shared.0.lock().unwrap().snapshot_req;
        if want && have_latest {
            let mut data = vec![0u8; (w * h * 4) as usize];
            unsafe {
                ctx.CopyResource(&full, &latest);
                let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                ctx.Map(&full, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
                for y in 0..h as usize {
                    std::ptr::copy_nonoverlapping(
                        (m.pData as *const u8).add(y * m.RowPitch as usize),
                        data.as_mut_ptr().add(y * w as usize * 4),
                        w as usize * 4,
                    );
                }
                ctx.Unmap(&full, 0);
            }
            let (mtx, cv) = &**shared;
            let mut s = mtx.lock().unwrap();
            s.snapshot_req = false;
            s.snapshot = Some((w, h, data));
            cv.notify_all();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 输入注入

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    assert_eq!(n as usize, inputs.len(), "SendInput 被拦截");
}

fn send_hotkey() {
    send(&[
        key(VK_CONTROL, false),
        key(VK_MENU, false),
        key(VK_SHIFT, false),
        key(VK_F9, false),
        key(VK_F9, true),
        key(VK_SHIFT, true),
        key(VK_MENU, true),
        key(VK_CONTROL, true),
    ]);
}

fn mouse(x: i32, y: i32, extra: MOUSE_EVENT_FLAGS) -> INPUT {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: (((x - vx) as i64 * 65535 + (vw as i64 - 1) / 2) / (vw as i64 - 1)) as i32,
                dy: (((y - vy) as i64 * 65535 + (vh as i64 - 1) / 2) / (vh as i64 - 1)) as i32,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | extra,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn window_pid(hwnd: windows::Win32::Foundation::HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

// ---------------------------------------------------------------- 子进程

struct Proto {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    log: Arc<Mutex<File>>,
}

impl Proto {
    fn spawn(exe: &Path, args: &[String], log: Arc<Mutex<File>>, stderr: File) -> Self {
        let mut child = Command::new(exe)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("启动原型失败");
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take();
        let (tx, rx) = channel();
        let log2 = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let _ = writeln!(log2.lock().unwrap(), "  < {line}");
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines: rx,
            log,
        }
    }

    fn wait_line(&self, prefix: &str, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) if l.starts_with(prefix) => return Some(l),
                Ok(l) if l.starts_with("ERROR") => {
                    self.note(&format!("原型报错：{l}"));
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    return None;
                }
            }
        }
    }

    fn cmd(&mut self, c: &str) {
        if let Some(s) = self.stdin.as_mut() {
            let _ = writeln!(s, "{c}");
            let _ = s.flush();
        }
    }

    fn note(&self, s: &str) {
        let _ = writeln!(self.log.lock().unwrap(), "  ! {s}");
    }

    fn memory(&self) -> PROCESS_MEMORY_COUNTERS_EX2 {
        let mut m = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                HANDLE(self.child.as_raw_handle()),
                &mut m as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                m.cb,
            )
            .expect("GetProcessMemoryInfo");
        }
        m
    }

    fn finish(mut self) {
        self.cmd("quit");
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.note("原型未按时退出，强制结束");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

fn stamps(line: &str) -> Vec<(String, i64)> {
    line.split_whitespace()
        .filter_map(|t| {
            let (k, v) = t.split_once('=')?;
            Some((k.strip_prefix("t_")?.to_string(), v.parse().ok()?))
        })
        .collect()
}

// ---------------------------------------------------------------- 统计

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = ((p / 100.0) * s.len() as f64).ceil().max(1.0) as usize;
    s[rank.min(s.len()) - 1]
}

struct Trigger {
    first: bool,
    e2e: Option<f64>,
    stale: u32,
    phases: Vec<(String, f64)>,
}

struct MemRow {
    when: &'static str,
    ws: f64,
    pws: f64,
    private: f64,
}

struct ConfigResult {
    label: String,
    triggers: Vec<Trigger>,
    mems: Vec<MemRow>,
    startup: Vec<f64>,
    init: Vec<f64>,
    drag: String,
}

fn mb(b: usize) -> f64 {
    b as f64 / 1024.0 / 1024.0
}

// ---------------------------------------------------------------- 主流程

struct Opts {
    out: PathBuf,
    protos: Vec<(String, PathBuf)>,
    strategies: Vec<String>,
    cold_runs: usize,
    triggers: usize,
    hold_ms: u64,
    drag: bool,
}

fn parse() -> Opts {
    let mut o = Opts {
        out: PathBuf::from("results"),
        protos: Vec::new(),
        strategies: vec!["prewarm".into(), "ondemand".into(), "lazy".into()],
        cold_runs: 8,
        triggers: 12,
        hold_ms: 250,
        drag: true,
    };
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--out" => o.out = PathBuf::from(&a[i + 1]),
            "--proto" => {
                let (name, exe) = a[i + 1].split_once('=').expect("--proto 名称=exe");
                o.protos.push((name.into(), PathBuf::from(exe)));
            }
            "--strategies" => o.strategies = a[i + 1].split(',').map(String::from).collect(),
            "--cold-runs" => o.cold_runs = a[i + 1].parse().unwrap(),
            "--triggers" => o.triggers = a[i + 1].parse().unwrap(),
            "--hold-ms" => o.hold_ms = a[i + 1].parse().unwrap(),
            "--no-drag" => {
                o.drag = false;
                i += 1;
                continue;
            }
            s => panic!("未知参数 {s}"),
        }
        i += 2;
    }
    o
}

fn rand_ms(seed: &mut u64, lo: u64, hi: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    lo + (*seed >> 33) % (hi - lo)
}

fn save_png(path: &Path, w: u32, h: u32, bgra: &[u8]) {
    // 2× 降采样，避免 4K PNG 过大。
    let (ow, oh) = (w / 2, h / 2);
    let mut rgb = Vec::with_capacity((ow * oh * 3) as usize);
    for y in 0..oh {
        for x in 0..ow {
            let i = (((y * 2) * w + x * 2) * 4) as usize;
            rgb.extend_from_slice(&[bgra[i + 2], bgra[i + 1], bgra[i]]);
        }
    }
    let f = File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), ow, oh);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&rgb).unwrap();
}

/// 一个块内 BGR 的平均绝对差。
fn block_diff(a: &[u8], b: &[u8], w: u32, x0: u32, y0: u32, size: u32) -> f64 {
    let mut sum = 0u64;
    for y in y0..y0 + size {
        for x in x0..x0 + size {
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                sum += (a[i + c] as i32 - b[i + c] as i32).unsigned_abs() as u64;
            }
        }
    }
    sum as f64 / (size * size * 3) as f64
}

fn drag_test(
    exe: &Path,
    strategy: &str,
    out: &Path,
    label: &str,
    watcher: &Watcher,
    log: &Arc<Mutex<File>>,
    stderr: &File,
) -> String {
    let args = vec![
        "--strategy".into(),
        strategy.into(),
        "--hold-ms".into(),
        "250".into(),
        "--harness".into(),
        "--exit-after-s".into(),
        "60".into(),
    ];
    let mut p = Proto::spawn(exe, &args, log.clone(), stderr.try_clone().unwrap());
    if p.wait_line("READY", Duration::from_secs(20)).is_none() {
        p.finish();
        return "失败：未就绪".into();
    }
    std::thread::sleep(Duration::from_millis(800));
    let pid = p.child.id();
    let baseline = watcher.snapshot();
    p.cmd("hold");
    std::thread::sleep(Duration::from_millis(100));
    watcher.arm(proto::sentinel_code(1));
    send_hotkey();
    let (found, _) = watcher.wait_found(Duration::from_secs(3));
    watcher.disarm();
    let ev = p.wait_line("EVENT", Duration::from_secs(3));
    if found.is_none() || ev.is_none() {
        p.cmd("close");
        p.finish();
        return "失败：遮罩未出现".into();
    }
    std::thread::sleep(Duration::from_millis(200));
    let mut saved = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut saved);
    }
    // 主显示器上 30%→60% 的对角拖拽（物理像素）。
    let (pw, ph) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let (x0, y0, x1, y1) = (pw * 3 / 10, ph * 3 / 10, pw * 6 / 10, ph * 55 / 100);
    let under = unsafe { WindowFromPoint(POINT { x: x0, y: y0 }) };
    let mut result = String::new();
    if window_pid(under) != pid {
        result.push_str("跳过拖拽：起点下不是原型窗口；");
    } else {
        send(&[mouse(x0, y0, MOUSE_EVENT_FLAGS(0))]);
        std::thread::sleep(Duration::from_millis(40));
        send(&[mouse(x0, y0, MOUSEEVENTF_LEFTDOWN)]);
        for i in 1..=12 {
            std::thread::sleep(Duration::from_millis(16));
            let x = x0 + (x1 - x0) * i / 12;
            let y = y0 + (y1 - y0) * i / 12;
            send(&[mouse(x, y, MOUSE_EVENT_FLAGS(0))]);
        }
        std::thread::sleep(Duration::from_millis(30));
        send(&[mouse(x1, y1, MOUSEEVENTF_LEFTUP)]);
        match p.wait_line("SELECTION", Duration::from_secs(2)) {
            Some(sel) => {
                let g = |k| {
                    kv(&sel, k)
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(-1)
                };
                let (sx, sy, sw, sh) = (g("x"), g("y"), g("w"), g("h"));
                let ok = (sx - x0).abs() <= 3
                    && (sy - y0).abs() <= 3
                    && (sw - (x1 - x0)).abs() <= 3
                    && (sh - (y1 - y0)).abs() <= 3;
                result.push_str(&format!(
                    "选区 {sx},{sy} {sw}×{sh}（期望 {x0},{y0} {}×{}）{}；",
                    x1 - x0,
                    y1 - y0,
                    if ok { "一致" } else { "不一致" }
                ));
                std::thread::sleep(Duration::from_millis(300));
                if let Some((w, h, img)) = watcher.snapshot() {
                    let png = out.join(format!("{label}-drag.png"));
                    save_png(&png, w, h, &img);
                    result.push_str(&format!(
                        "截图 {}；",
                        png.file_name().unwrap().to_string_lossy()
                    ));
                    if let Some((bw, _, base)) = &baseline
                        && *bw == w
                    {
                        // 选区内应与冻结前的桌面逐像素一致（只要桌面本身没变）。
                        let cx = ((x0 + x1) / 2 - 32) as u32;
                        let cy = ((y0 + y1) / 2 - 32) as u32;
                        let inside = block_diff(base, &img, w, cx, cy, 64);
                        result.push_str(&format!("选区内与原桌面平均色差 {inside:.2}；"));
                    }
                }
            }
            None => result.push_str("未收到 SELECTION；"),
        }
    }
    unsafe {
        let _ = SetCursorPos(saved.x, saved.y);
    }
    let fg = unsafe { GetForegroundWindow() };
    if window_pid(fg) == pid {
        send(&[key(VK_ESCAPE, false), key(VK_ESCAPE, true)]);
        match p.wait_line("CLOSED", Duration::from_secs(2)) {
            Some(l) => result.push_str(&format!("Esc 关闭：{}", kv(&l, "reason").unwrap_or("?"))),
            None => result.push_str("Esc 未关闭"),
        }
    } else {
        result.push_str("遮罩不在前台，改用命令关闭");
        p.cmd("close");
        let _ = p.wait_line("CLOSED", Duration::from_secs(2));
    }
    p.finish();
    result
}

fn run_config(
    o: &Opts,
    name: &str,
    exe: &Path,
    strategy: &str,
    watcher: &Watcher,
    log: &Arc<Mutex<File>>,
    seed: &mut u64,
) -> ConfigResult {
    let label = format!("{name}-{strategy}");
    let stderr = File::create(o.out.join(format!("{label}.stderr.log"))).unwrap();
    let _ = writeln!(log.lock().unwrap(), "== {label}");
    let mut r = ConfigResult {
        label: label.clone(),
        triggers: Vec::new(),
        mems: Vec::new(),
        startup: Vec::new(),
        init: Vec::new(),
        drag: String::new(),
    };
    for run in 0..o.cold_runs {
        let args = vec![
            "--strategy".into(),
            strategy.into(),
            "--hold-ms".into(),
            o.hold_ms.to_string(),
            "--harness".into(),
            "--exit-after-s".into(),
            "120".into(),
        ];
        let t_spawn = qpc();
        let p = Proto::spawn(exe, &args, log.clone(), stderr.try_clone().unwrap());
        let Some(ready) = p.wait_line("READY", Duration::from_secs(20)) else {
            p.note("未就绪");
            p.finish();
            continue;
        };
        r.startup.push(ms(qpc() - t_spawn));
        if let Some(v) = kv(&ready, "init_ms").and_then(|v| v.parse().ok()) {
            r.init.push(v);
        }
        if run == 0 {
            let _ = writeln!(log.lock().unwrap(), "  {ready}");
        }
        std::thread::sleep(Duration::from_secs(2));
        let m = p.memory();
        r.mems.push(MemRow {
            when: "idle_before",
            ws: mb(m.WorkingSetSize),
            pws: mb(m.PrivateWorkingSetSize),
            private: mb(m.PrivateUsage),
        });
        for i in 0..=o.triggers {
            let n = (i + 1) as u32;
            watcher.arm(proto::sentinel_code(n));
            let t0 = qpc();
            send_hotkey();
            let (found, stale) = watcher.wait_found(Duration::from_secs(3));
            watcher.disarm();
            let trace = watcher.take_trace();
            let tr: Vec<String> = trace
                .iter()
                .skip(trace.len().saturating_sub(6))
                .map(|(t, r, g, b)| format!("{:.1}ms:{r},{g},{b}", ms(t - t0)))
                .collect();
            p.note(&format!("trigger {n} 帧轨迹（最后 6 帧）{}", tr.join(" ")));
            if found.is_none() {
                // 诊断：换 GDI 当第二个观察者，看哨兵是否其实在屏幕上（需配合较长的 --hold-ms）。
                let px = unsafe {
                    let dc = windows::Win32::Graphics::Gdi::GetDC(None);
                    let c = windows::Win32::Graphics::Gdi::GetPixel(dc, 4, 4);
                    windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
                    c.0
                };
                p.note(&format!(
                    "未上屏，GDI 读 (4,4) = {},{},{}（期望 255,{},255）",
                    px & 0xFF,
                    (px >> 8) & 0xFF,
                    (px >> 16) & 0xFF,
                    proto::sentinel_code(n)
                ));
                // 再把观察者最后拿到的画面存下来。
                if let Some((w, h, img)) = watcher.snapshot() {
                    let png = o.out.join(format!("{label}-missing-r{run}-n{n}.png"));
                    save_png(&png, w, h, &img);
                    p.note(&format!("未上屏，截图 {}", png.display()));
                }
            }
            let ev = p.wait_line("EVENT", Duration::from_secs(3));
            let mut phases = Vec::new();
            if let Some(ev) = &ev {
                let mut prev = t0;
                for (k, v) in stamps(ev) {
                    phases.push((k, ms(v - prev)));
                    prev = v;
                }
                if let Some(f) = found {
                    phases.push(("visible".into(), ms(f - prev)));
                }
            } else {
                p.note(&format!("第 {n} 次触发没有 EVENT"));
            }
            r.triggers.push(Trigger {
                first: i == 0,
                e2e: found.map(|f| ms(f - t0)),
                stale,
                phases,
            });
            if p.wait_line("CLOSED", Duration::from_millis(o.hold_ms + 9000))
                .is_none()
            {
                p.note("遮罩未关闭");
            }
            std::thread::sleep(Duration::from_millis(rand_ms(seed, 250, 450)));
        }
        std::thread::sleep(Duration::from_secs(2));
        let m = p.memory();
        r.mems.push(MemRow {
            when: "idle_after",
            ws: mb(m.WorkingSetSize),
            pws: mb(m.PrivateWorkingSetSize),
            private: mb(m.PrivateUsage),
        });
        p.finish();
        std::thread::sleep(Duration::from_millis(500));
    }
    if o.drag {
        r.drag = drag_test(exe, strategy, &o.out, &label, watcher, log, &stderr);
    }
    r
}

fn fmt(v: f64) -> String {
    if v.is_nan() {
        "-".into()
    } else {
        format!("{v:.1}")
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).unwrap();
    }
    let o = parse();
    std::fs::create_dir_all(&o.out).unwrap();
    let log = Arc::new(Mutex::new(File::create(o.out.join("harness.log")).unwrap()));
    let watcher = Watcher::start();
    let mut seed = qpc() as u64;
    let mut results = Vec::new();
    for (name, exe) in &o.protos {
        for s in &o.strategies {
            eprintln!("测量 {name}-{s} ...");
            let r = run_config(&o, name, exe, s, &watcher, &log, &mut seed);
            eprintln!("  完成：{} 次触发", r.triggers.len());
            results.push(r);
        }
    }
    drop(watcher);

    // 原始数据
    let mut csv = File::create(o.out.join("triggers.csv")).unwrap();
    writeln!(csv, "config,kind,e2e_ms,stale_frames,phases").unwrap();
    for r in &results {
        for t in &r.triggers {
            let ph: Vec<String> = t
                .phases
                .iter()
                .map(|(k, v)| format!("{k}={v:.2}"))
                .collect();
            writeln!(
                csv,
                "{},{},{},{},{}",
                r.label,
                if t.first { "first" } else { "next" },
                t.e2e.map(|v| format!("{v:.2}")).unwrap_or_default(),
                t.stale,
                ph.join("|")
            )
            .unwrap();
        }
    }
    let mut mcsv = File::create(o.out.join("memory.csv")).unwrap();
    writeln!(
        mcsv,
        "config,when,working_set_mb,private_working_set_mb,private_bytes_mb"
    )
    .unwrap();
    for r in &results {
        for m in &r.mems {
            writeln!(
                mcsv,
                "{},{},{:.2},{:.2},{:.2}",
                r.label, m.when, m.ws, m.pws, m.private
            )
            .unwrap();
        }
    }

    // 汇总
    let mut md = String::new();
    md.push_str(
        "## 热键→遮罩上屏延迟（ms，端到端：SendInput → 含哨兵的桌面帧 LastPresentTime）\n\n",
    );
    md.push_str("| 配置 | 首次 中位数 | 首次 P95 | 首次 样本 | 后续 中位数 | 后续 P95 | 后续 样本 | 未上屏 | 旧帧 |\n|---|---|---|---|---|---|---|---|---|\n");
    for r in &results {
        let sel = |first: bool| -> Vec<f64> {
            r.triggers
                .iter()
                .filter(|t| t.first == first)
                .filter_map(|t| t.e2e)
                .collect()
        };
        let (f, n) = (sel(true), sel(false));
        let missing = r.triggers.iter().filter(|t| t.e2e.is_none()).count();
        let stale: u32 = r.triggers.iter().map(|t| t.stale).sum();
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.label,
            fmt(pct(&f, 50.0)),
            fmt(pct(&f, 95.0)),
            f.len(),
            fmt(pct(&n, 50.0)),
            fmt(pct(&n, 95.0)),
            n.len(),
            missing,
            stale
        ));
    }
    md.push_str(
        "\n## 阶段耗时中位数（ms，相邻时间戳之差；input = SendInput→进程收到 WM_HOTKEY）\n\n",
    );
    for r in &results {
        for first in [true, false] {
            let mut keys: Vec<String> = Vec::new();
            for t in r.triggers.iter().filter(|t| t.first == first) {
                for (k, _) in &t.phases {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
            }
            let cells: Vec<String> = keys
                .iter()
                .map(|k| {
                    let v: Vec<f64> = r
                        .triggers
                        .iter()
                        .filter(|t| t.first == first)
                        .filter_map(|t| t.phases.iter().find(|(kk, _)| kk == k).map(|(_, v)| *v))
                        .collect();
                    let name = if k == "hotkey" { "input" } else { k.as_str() };
                    format!("{name} {}", fmt(pct(&v, 50.0)))
                })
                .collect();
            md.push_str(&format!(
                "- {} {}：{}\n",
                r.label,
                if first { "首次" } else { "后续" },
                cells.join(" · ")
            ));
        }
    }
    md.push_str("\n## 内存（MB，中位数；idle_before = 启动后空闲 2 s，idle_after = 触发完毕、遮罩关闭后空闲 2 s）\n\n");
    md.push_str("| 配置 | 时机 | 工作集 | 私有工作集 | 私有字节 |\n|---|---|---|---|---|\n");
    for r in &results {
        for when in ["idle_before", "idle_after"] {
            let col = |f: fn(&MemRow) -> f64| -> f64 {
                let v: Vec<f64> = r.mems.iter().filter(|m| m.when == when).map(f).collect();
                pct(&v, 50.0)
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                r.label,
                when,
                fmt(col(|m| m.ws)),
                fmt(col(|m| m.pws)),
                fmt(col(|m| m.private))
            ));
        }
    }
    md.push_str(
        "\n## 启动（ms，中位数）\n\n| 配置 | 进程启动→READY | 其中初始化 |\n|---|---|---|\n",
    );
    for r in &results {
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            r.label,
            fmt(pct(&r.startup, 50.0)),
            fmt(pct(&r.init, 50.0))
        ));
    }
    md.push_str("\n## 拖拽框选验证\n\n");
    for r in &results {
        md.push_str(&format!("- {}：{}\n", r.label, r.drag));
    }
    std::fs::write(o.out.join("summary.md"), &md).unwrap();
    print!("{md}");
}
