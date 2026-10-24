//! BetterPrtSc 常驻实例（`BetterPrtSc.exe`）：托盘图标 + 全局热键 → `core` 命令。
//!
//! 这里只做组装：创建平台事件循环和平台能力，交给 `bps_core::ResidentInstance`，
//! 再把事件循环递交的每个平台事件交给它处理。

// release 版不弹控制台窗口；debug 版保留控制台，便于查看输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod icon;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    match resident::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("BetterPrtSc 启动失败：{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("BetterPrtSc 目前只支持 Windows");
    ExitCode::FAILURE
}

#[cfg(windows)]
mod resident {
    use std::ops::ControlFlow;
    use std::time::Instant;

    use bps_core::{Flow, ResidentInstance, default_bindings};
    use bps_platform::PlatformError;
    use bps_platform_windows::{EventLoop, dpi};

    use crate::icon;

    pub fn run() -> Result<(), PlatformError> {
        let declared_by_manifest = dpi::is_per_monitor_v2();
        if !dpi::ensure_per_monitor_v2() {
            eprintln!("警告：未能声明 Per-Monitor V2 DPI 感知，截图可能不是物理像素");
        }
        eprintln!(
            "DPI 感知：PerMonitorV2（{}）",
            if declared_by_manifest {
                "应用清单"
            } else {
                "运行时声明"
            }
        );

        let event_loop = EventLoop::new()?;
        event_loop.show_tray_icon("BetterPrtSc", &icon::tray_icon())?;

        let mut instance = ResidentInstance::new(
            bps_platform_windows::platform(&event_loop),
            default_bindings(),
        );
        for failure in instance.register_hotkeys() {
            eprintln!(
                "热键 {:?} 注册失败：{}",
                failure.binding.hotkey, failure.error
            );
        }
        eprintln!("常驻实例已启动");

        event_loop.run(|event| {
            let started = Instant::now();
            match instance.handle(event) {
                Ok(Flow::Continue) => {
                    eprintln!("已处理 {event:?}，耗时 {:?}", started.elapsed());
                    ControlFlow::Continue(())
                }
                Ok(Flow::Exit) => ControlFlow::Break(()),
                Err(e) => {
                    eprintln!("处理 {event:?} 失败：{e}");
                    ControlFlow::Continue(())
                }
            }
        });
        eprintln!("常驻实例已退出");
        Ok(())
    }
}
