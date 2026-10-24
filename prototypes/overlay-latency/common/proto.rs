//! 一次性原型代码（ticket #2）：两个遮罩原型共用的计时、参数与 stdout 协议。
//! 通过 `#[path = "../../common/proto.rs"]` 引入，不是共享 crate。

#![allow(dead_code)]

use std::io::{BufRead, Write};

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F9,
};

/// 测量用热键：Ctrl+Alt+Shift+F9（避开 PrtSc，避免和用户正在用的截图工具冲突）。
pub const HOTKEY_ID: i32 = 0x0B25;
pub const HOTKEY_MODS: HOT_KEY_MODIFIERS =
    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
pub const HOTKEY_VK: u32 = VK_F9.0 as u32;

/// 哨兵方块边长（物理像素），画在主显示器左上角，供 harness 用 DXGI 观察遮罩何时真正上屏。
pub const SENTINEL_SIZE: i32 = 8;

/// 第 n 次触发的哨兵绿色分量；相邻触发相差 37，便于识别“旧帧”。
pub fn sentinel_code(n: u32) -> u8 {
    ((n * 37) % 200 + 28) as u8
}

/// 自动关闭前的最长显示时间（安全阀）。
pub const SAFETY_CLOSE_MS: u64 = 8000;

pub fn qpc() -> i64 {
    let mut v = 0i64;
    unsafe { QueryPerformanceCounter(&mut v).ok() };
    v
}

pub fn qpf() -> i64 {
    let mut v = 0i64;
    unsafe { QueryPerformanceFrequency(&mut v).ok() };
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// 启动时建好 GPU 设备、渲染器、隐藏（cloak）的遮罩窗口与交换链，并完整预热一遍。
    Prewarm,
    /// 启动时只建 GPU 设备与渲染器；每次触发创建窗口与交换链，关闭时销毁。
    OnDemand,
    /// 启动时不碰 GPU；每次触发从零创建全部资源，关闭后全部释放。
    Lazy,
}

impl Strategy {
    pub fn name(self) -> &'static str {
        match self {
            Strategy::Prewarm => "prewarm",
            Strategy::OnDemand => "ondemand",
            Strategy::Lazy => "lazy",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Args {
    pub strategy: Strategy,
    /// >0：遮罩显示这么久后自动关闭（测量模式）；0：只在 Esc / 右键时关闭（手动模式）。
    pub hold_ms: u64,
    /// 从 stdin 读控制命令（hold / close / quit），stdin 关闭即退出。
    pub harness: bool,
    /// 进程最长存活时间，防止遗留进程。
    pub exit_after_s: u64,
}

pub fn parse_args() -> Args {
    let mut a = Args {
        strategy: Strategy::Prewarm,
        hold_ms: 0,
        harness: false,
        exit_after_s: 120,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--strategy" => {
                i += 1;
                a.strategy = match argv[i].as_str() {
                    "prewarm" => Strategy::Prewarm,
                    "ondemand" => Strategy::OnDemand,
                    "lazy" => Strategy::Lazy,
                    s => panic!("未知策略 {s}"),
                }
            }
            "--hold-ms" => {
                i += 1;
                a.hold_ms = argv[i].parse().unwrap();
            }
            "--harness" => a.harness = true,
            "--exit-after-s" => {
                i += 1;
                a.exit_after_s = argv[i].parse().unwrap();
            }
            s => panic!("未知参数 {s}"),
        }
        i += 1;
    }
    a
}

#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    /// 下一次触发不自动关闭（用于拖拽框选验证）。
    Hold,
    /// 立即关闭遮罩。
    Close,
    /// 退出进程。
    Quit,
}

/// 读 stdin 控制命令；stdin 关闭（harness 退出）时发出 Quit，保证不遗留进程。
pub fn spawn_stdin_reader(send: impl Fn(Cmd) + Send + 'static) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            match line.trim() {
                "hold" => send(Cmd::Hold),
                "close" => send(Cmd::Close),
                "quit" => {
                    send(Cmd::Quit);
                    return;
                }
                _ => {}
            }
        }
        send(Cmd::Quit);
    });
}

pub fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// 一次触发的时间线：按发生顺序记录 (阶段名, QPC)。harness 用相邻差值得到各阶段耗时。
pub struct Timeline {
    pub n: u32,
    stamps: Vec<(&'static str, i64)>,
}

impl Timeline {
    pub fn new(n: u32, t_hotkey: i64) -> Self {
        Self {
            n,
            stamps: vec![("hotkey", t_hotkey)],
        }
    }
    pub fn mark(&mut self, name: &'static str) {
        self.stamps.push((name, qpc()));
    }
    pub fn emit(&self, extra: &str) {
        let mut s = format!("EVENT n={}", self.n);
        for (k, v) in &self.stamps {
            s.push_str(&format!(" t_{k}={v}"));
        }
        if !extra.is_empty() {
            s.push(' ');
            s.push_str(extra);
        }
        emit(&s);
    }
}
