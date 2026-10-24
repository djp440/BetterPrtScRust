//! BetterPrtSc 常驻实例（`BetterPrtSc.exe`）：托盘图标 + 全局热键 → `core` 命令。
//!
//! 这里只做组装：决定配置文件的位置（便携模式或 `%APPDATA%`），创建平台事件循环和
//! 平台能力，交给 `bps_core::ResidentInstance`，再把事件循环递交的每个平台事件交给它处理。

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
    use std::path::PathBuf;
    use std::time::Instant;

    use bps_core::{DataLocation, Flow, PORTABLE_MARKER, ResidentInstance};
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

        let location = data_location()?;
        eprintln!(
            "配置文件：{}{}",
            location.config_file().display(),
            if location.is_portable() {
                "（便携模式）"
            } else {
                ""
            }
        );

        let event_loop = EventLoop::new()?;
        // 先显示托盘图标：启动过程中的通知（如热键注册失败）要靠它发出。
        event_loop.show_tray_icon("BetterPrtSc", &icon::tray_icon())?;

        let mut instance = ResidentInstance::start(bps_platform_windows::platform(
            &event_loop,
            location.config_file(),
        ));
        for failure in instance.registration_failures() {
            eprintln!(
                "热键 {} 注册失败：{}",
                failure.binding.hotkey, failure.error
            );
        }
        eprintln!(
            "常驻实例已启动，{} 条热键绑定",
            instance.config().bindings.len()
        );

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

    /// 程序目录下有 `portable` 标记文件时用便携模式，否则用 `%APPDATA%\BetterPrtSc`。
    fn data_location() -> Result<DataLocation, PlatformError> {
        let exe = std::env::current_exe()
            .map_err(|e| PlatformError::new(format!("无法确定程序所在目录：{e}")))?;
        let program_dir = exe
            .parent()
            .ok_or_else(|| PlatformError::new("无法确定程序所在目录"))?;
        let portable = program_dir.join(PORTABLE_MARKER).is_file();
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
        DataLocation::resolve(program_dir, portable, app_data.as_deref()).ok_or_else(|| {
            PlatformError::new(format!(
                "找不到 %APPDATA%；可以在程序目录下放一个名为 {PORTABLE_MARKER} 的文件，改用便携模式"
            ))
        })
    }
}
