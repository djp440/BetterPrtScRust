//! `bps` CLI：把命令经命名管道转交给 BetterPrtSc 常驻实例，见 `bps --help`。
//!
//! 解析、转交和退出码都在 `bps_core::run_cli` 里；这里只接上 Windows 的命名管道通道。
//! 常驻实例没在运行时，拉起与本程序同目录的 `BetterPrtSc.exe`。

use std::process::ExitCode;

#[cfg(windows)]
mod location;

#[cfg(windows)]
fn main() -> ExitCode {
    use bps_platform_windows::{PipeChannel, resident_pipe_name};

    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let resident_exe = std::env::current_exe()
        .map(|exe| exe.with_file_name("BetterPrtSc.exe"))
        .unwrap_or_else(|_| "BetterPrtSc.exe".into());
    let mut channel = PipeChannel::new(&resident_pipe_name(), resident_exe);

    // 输出语言与常驻实例一致：读不到配置文件时按系统语言。
    let config_text = location::data_location()
        .ok()
        .and_then(|location| std::fs::read_to_string(location.config_file()).ok());
    let locale = bps_core::cli_locale(
        config_text.as_deref(),
        &bps_platform_windows::user_ui_languages(),
    );
    let outcome = bps_core::run_cli(&args, &mut channel, locale);
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    ExitCode::from(outcome.status.exit_code())
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("bps 目前只支持 Windows");
    ExitCode::FAILURE
}
