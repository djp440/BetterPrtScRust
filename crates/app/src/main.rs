//! BetterPrtSc 常驻实例（`BetterPrtSc.exe`）：托盘图标 + 全局热键 + 命名管道 → `core` 命令。
//!
//! 这里只做组装：先占用常驻实例的命名管道（单实例），再决定配置文件的位置（便携模式或
//! `%APPDATA%`），创建平台事件循环和平台能力，交给 `bps_core::ResidentInstance`，
//! 然后把事件循环递交的每个平台事件交给它处理。
//! 管道已被占用时说明常驻实例已在运行，本进程只把“激活”请求转交给它，随即退出。
//!
//! 命令行参数（都由程序自己或安装包使用，不面向用户）：
//! - `--restarted-as-administrator`：常驻实例为“以管理员身份运行”重新启动了自己，
//!   新进程等旧进程让出命名管道后接替它；
//! - `--remove-autostart`：卸载前移除指向本程序的开机自启（安装包的卸载流程调用），
//!   移除计划任务需要管理员权限时以管理员身份再运行一次（弹出 UAC 提示）；
//! - `--settings`：本程序也是独立窗口界面的子进程，打开设置界面（见 [`settings`]），
//!   用户关掉窗口即退出。由常驻实例按需启动；常驻实例本身不初始化任何界面库；
//! - `--result-panel`：打开结果面板，识别常驻实例经标准输入交来的截图里的文字（见
//!   [`result_panel`]），用户关掉窗口即退出。

// release 版不弹控制台窗口；debug 版保留控制台，便于查看输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod location;
#[cfg(windows)]
mod result_panel;
#[cfg(windows)]
mod settings;

use std::process::ExitCode;

#[cfg(windows)]
use location::data_location;

#[cfg(windows)]
fn main() -> ExitCode {
    use bps_platform_windows::{RESULT_PANEL_ARGUMENT, SETTINGS_ARGUMENT};

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == resident::REMOVE_AUTOSTART_ARG) {
        return resident::remove_autostart_before_uninstall();
    }
    let result = if args.first().is_some_and(|a| a == SETTINGS_ARGUMENT) {
        settings::run()
    } else if args.first().is_some_and(|a| a == RESULT_PANEL_ARGUMENT) {
        result_panel::run()
    } else {
        let restarted = args
            .iter()
            .any(|a| a == bps_platform_windows::RESTARTED_AS_ADMINISTRATOR_ARG);
        resident::run(restarted)
    };
    match result {
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
    use std::process::ExitCode;
    use std::thread;
    use std::time::{Duration, Instant};

    use bps_core::{
        Flow, Reply, Request, ResidentInstance, StartOptions, WhenNotRunning, forward,
        remove_autostart,
    };
    use bps_platform::{Autostart, AutostartMethod, PlatformError, PlatformEvent};
    use bps_platform_windows::{
        EventLoop, PipeChannel, PipeClaim, ResidentPipe, WindowsAutostart,
        allow_any_process_to_set_foreground, dpi, is_elevated, resident_pipe_name,
        run_self_as_administrator_and_wait,
    };

    use crate::{data_location, icon};

    /// 卸载前移除开机自启的命令行参数。
    pub const REMOVE_AUTOSTART_ARG: &str = "--remove-autostart";
    /// 以管理员身份重新启动后，等旧的常驻实例让出命名管道的最长时间。
    const HANDOVER_TIMEOUT: Duration = Duration::from_secs(10);

    /// `restarted`：本进程是常驻实例为“以管理员身份运行”而重新启动的。
    pub fn run(restarted: bool) -> Result<(), Box<dyn Error>> {
        // 最先占用命名管道：已有常驻实例时不显示托盘图标、不注册热键。
        let pipe_name = resident_pipe_name();
        let pipe = match claim_pipe(&pipe_name, restarted)? {
            Some(pipe) => pipe,
            None => return activate_running_instance(&pipe_name),
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
        let mut instance = ResidentInstance::start_with(
            bps_platform_windows::platform(&event_loop, location.config_file(), ui_exe),
            StartOptions {
                portable: location.is_portable(),
                restarted_as_administrator: restarted,
            },
        );
        if instance.restarting_as_administrator() {
            eprintln!("已以管理员身份启动新的常驻实例，本进程退出");
            return Ok(());
        }
        eprintln!("权限：{}", if is_elevated() { "管理员" } else { "普通" });
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
            // 遮罩上的鼠标移动等输入很密集，只在出错时输出。
            let quiet = matches!(
                event,
                PlatformEvent::OverlayInput(_) | PlatformEvent::RegionSampled
            );
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

    /// 占用常驻实例的命名管道；已有常驻实例时返回 `None`。
    ///
    /// 以管理员身份重新启动的进程要等旧的常驻实例退出、让出管道，最多等 [`HANDOVER_TIMEOUT`]。
    fn claim_pipe(pipe_name: &str, restarted: bool) -> Result<Option<ResidentPipe>, PlatformError> {
        let deadline = Instant::now() + HANDOVER_TIMEOUT;
        loop {
            match ResidentPipe::claim(pipe_name)? {
                PipeClaim::Claimed(pipe) => return Ok(Some(pipe)),
                PipeClaim::AlreadyRunning if restarted && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(100));
                }
                PipeClaim::AlreadyRunning => return Ok(None),
            }
        }
    }

    /// 卸载前移除指向本程序的开机自启：注册表 Run 项直接移除；以管理员身份运行时登记的
    /// 计划任务如果因为权限不足移除不了，以管理员身份再运行一次本命令（弹出 UAC 提示）。
    pub fn remove_autostart_before_uninstall() -> ExitCode {
        let mut autostart = WindowsAutostart::new();
        let Err(e) = remove_autostart(&mut autostart) else {
            return ExitCode::SUCCESS;
        };
        eprintln!("移除开机自启失败：{e}");
        let task_left = autostart.entries().is_ok_and(|entries| {
            entries
                .iter()
                .any(|e| e.this_program && e.method == AutostartMethod::Elevated)
        });
        if !task_left || is_elevated() {
            return ExitCode::FAILURE;
        }
        match run_self_as_administrator_and_wait(REMOVE_AUTOSTART_ARG) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(code) => {
                eprintln!("以管理员身份移除开机自启失败（退出码 {code}）");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        }
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
            Reply::Failed(message) => Err(format!("激活常驻实例失败：{message}").into()),
            Reply::Cancelled => Err("激活常驻实例失败：请求被取消".into()),
        }
    }
}
