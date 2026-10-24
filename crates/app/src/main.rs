//! BetterPrtSc 常驻实例（`BetterPrtSc.exe`）：托盘图标 + 全局热键 + 命名管道 → `core` 命令。
//!
//! 这里只做组装：先占用常驻实例的命名管道（单实例），再决定配置文件的位置（便携模式或
//! `%APPDATA%`），创建平台事件循环和平台能力，交给 `bps_core::ResidentInstance`，
//! 然后把事件循环递交的每个平台事件交给它处理。
//! 管道已被占用时说明常驻实例已在运行，本进程只把“激活”请求转交给它，随即退出。
//!
//! 同一个程序也是独立窗口界面的子进程：`BetterPrtSc.exe --settings` 打开设置界面
//! （见 [`settings`]），用户关掉窗口即退出。常驻实例本身不初始化任何界面库。

// release 版不弹控制台窗口；debug 版保留控制台，便于查看输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod settings;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use bps_platform::StandaloneWindow;
    use bps_platform_windows::window_argument;

    let first = std::env::args_os().nth(1);
    let settings = first.is_some_and(|arg| arg == window_argument(StandaloneWindow::Settings));
    let result = if settings {
        settings::run()
    } else {
        resident::run()
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("BetterPrtSc 启动失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 程序目录下有 `portable` 标记文件时用便携模式，否则用 `%APPDATA%\BetterPrtSc`。
/// 常驻实例和设置界面用同一套规则找配置文件。
#[cfg(windows)]
fn data_location() -> Result<bps_core::DataLocation, bps_platform::PlatformError> {
    use bps_core::{DataLocation, PORTABLE_MARKER};
    use bps_platform::PlatformError;

    let exe = std::env::current_exe()
        .map_err(|e| PlatformError::new(format!("无法确定程序所在目录：{e}")))?;
    let program_dir = exe
        .parent()
        .ok_or_else(|| PlatformError::new("无法确定程序所在目录"))?;
    let portable = program_dir.join(PORTABLE_MARKER).is_file();
    let app_data = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    DataLocation::resolve(program_dir, portable, app_data.as_deref()).ok_or_else(|| {
        PlatformError::new(format!(
            "找不到 %APPDATA%；可以在程序目录下放一个名为 {PORTABLE_MARKER} 的文件，改用便携模式"
        ))
    })
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

    use bps_core::{Flow, Reply, Request, ResidentInstance, WhenNotRunning, forward};
    use bps_platform_windows::{
        EventLoop, PipeChannel, PipeClaim, ResidentPipe, allow_any_process_to_set_foreground, dpi,
        resident_pipe_name,
    };

    use crate::{data_location, icon};

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

        // 设置界面等独立窗口用本程序自己以子进程打开。
        let ui_exe = std::env::current_exe()?;
        let mut instance = ResidentInstance::start(bps_platform_windows::platform(
            &event_loop,
            location.config_file(),
            ui_exe,
        ));
        for failure in instance.registration_failures() {
            eprintln!(
                "热键 {} 注册失败：{}",
                failure.binding.hotkey, failure.error
            );
        }
        pipe.serve(event_loop.sender())?;
        eprintln!(
            "常驻实例已启动（命名管道 {pipe_name}），{} 条热键绑定",
            instance.config().bindings.len()
        );

        event_loop.run(|event| {
            let started = Instant::now();
            let description = format!("{event:?}");
            match instance.handle(event) {
                Ok(Flow::Continue) => {
                    eprintln!("已处理 {description}，耗时 {:?}", started.elapsed());
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

    /// 再次启动时：把“激活”请求转交给已在运行的常驻实例（它会打开设置界面），不再拉起新的。
    fn activate_running_instance(pipe_name: &str) -> Result<(), Box<dyn Error>> {
        // 本进程是用户刚启动的，有前台权限；让给常驻实例，设置界面才能出现在最前面。
        allow_any_process_to_set_foreground();
        let mut channel = PipeChannel::new(pipe_name, std::env::current_exe()?);
        match forward(&Request::Activate, &mut channel, WhenNotRunning::GiveUp)? {
            Reply::Done | Reply::Status(_) => {
                eprintln!("BetterPrtSc 已在运行，已激活该常驻实例");
                Ok(())
            }
            Reply::Unsupported(message) | Reply::Failed(message) => {
                Err(format!("激活常驻实例失败：{message}").into())
            }
        }
    }
}
