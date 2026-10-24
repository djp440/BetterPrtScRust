//! BetterPrtSc 常驻实例（`BetterPrtSc.exe`）：托盘图标 + 全局热键 + 命名管道 → `core` 命令。
//!
//! 这里只做组装：先占用常驻实例的命名管道（单实例），再创建平台事件循环和平台能力，
//! 交给 `bps_core::ResidentInstance`，然后把事件循环递交的每个平台事件交给它处理。
//! 管道已被占用时说明常驻实例已在运行，本进程只把“激活”请求转交给它，随即退出。

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
    use std::error::Error;
    use std::ops::ControlFlow;
    use std::time::Instant;

    use bps_core::{
        Flow, Reply, Request, ResidentInstance, WhenNotRunning, default_bindings, forward,
    };
    use bps_platform::PlatformEvent;
    use bps_platform_windows::{
        EventLoop, PipeChannel, PipeClaim, ResidentPipe, dpi, resident_pipe_name,
    };

    use crate::icon;

    pub fn run() -> Result<(), Box<dyn Error>> {
        // 最先占用命名管道：已有常驻实例时不显示托盘图标、不注册热键。
        let pipe_name = resident_pipe_name();
        let pipe = match ResidentPipe::claim(&pipe_name)? {
            PipeClaim::Claimed(pipe) => pipe,
            PipeClaim::AlreadyRunning => return activate_running_instance(&pipe_name),
        };

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
        pipe.serve(event_loop.sender())?;
        eprintln!("常驻实例已启动（命名管道 {pipe_name}）");

        event_loop.run(|event| {
            let started = Instant::now();
            let description = format!("{event:?}");
            // 遮罩上的鼠标移动等输入很密集，只在出错时输出。
            let quiet = matches!(event, PlatformEvent::OverlayInput(_));
            match instance.handle(event) {
                Ok(Flow::Continue) => {
                    if !quiet {
                        eprintln!("已处理 {description}，耗时 {:?}", started.elapsed());
                    }
                    ControlFlow::Continue(())
                }
                Ok(Flow::Exit) => ControlFlow::Break(()),
                Err(e) => {
                    eprintln!("处理 {description} 失败：{e}");
                    ControlFlow::Continue(())
                }
            }
        });
        eprintln!("常驻实例已退出");
        Ok(())
    }

    /// 再次启动时：把“激活”请求转交给已在运行的常驻实例，不再拉起新的。
    fn activate_running_instance(pipe_name: &str) -> Result<(), Box<dyn Error>> {
        let mut channel = PipeChannel::new(pipe_name, std::env::current_exe()?);
        match forward(&Request::Activate, &mut channel, WhenNotRunning::GiveUp)? {
            Reply::Done => {
                eprintln!("BetterPrtSc 已在运行，已激活该常驻实例");
                Ok(())
            }
            Reply::Unsupported(message) | Reply::Failed(message) => {
                Err(format!("激活常驻实例失败：{message}").into())
            }
            Reply::Cancelled => Err("激活常驻实例失败：请求被取消".into()),
        }
    }
}
