//! 设置界面子进程（`BetterPrtSc.exe --settings`）：把 Windows 平台能力装配成
//! `bps_ui::SettingsEnvironment`，打开设置界面，用户关掉窗口即退出。
//!
//! 不占用常驻实例的命名管道，只经它查询常驻实例的状态；配置文件的位置与常驻实例相同。

use std::error::Error;
use std::path::Path;
use std::sync::Arc;

use bps_core::{Reply, Request, WhenNotRunning, forward};
use bps_platform_windows::start_key_capture;
use bps_platform_windows::{
    PipeChannel, map_system_font, open_with_default_program, pick_folder, read_clipboard_text,
    resident_pipe_name, standard_user_can_write, user_ui_languages, write_clipboard_text,
};
use bps_ui::{
    KeyStream, SettingsEnvironment, SystemClipboard, UiEnvironment, UiFont, WindowIcon,
    run_settings_window,
};

use crate::{data_location, icon};

pub fn run() -> Result<(), Box<dyn Error>> {
    let location = data_location()?;
    let exe = std::env::current_exe()?;
    let mut channel = PipeChannel::new(&resident_pipe_name(), exe.clone());
    let pause_exe = exe.clone();
    let environment = SettingsEnvironment {
        ui: ui_environment(),
        config_file: location.config_file(),
        query_status: Box::new(move || {
            match forward(&Request::Status, &mut channel, WhenNotRunning::GiveUp) {
                Ok(Reply::Status(status)) => Some(status),
                _ => None,
            }
        }),
        pick_folder: Arc::new(|title: &str, initial: Option<&Path>| pick_folder(title, initial)),
        key_capture: Arc::new(|| {
            let (inputs, guard) = start_key_capture().map_err(|e| e.to_string())?;
            Ok(KeyStream {
                inputs,
                guard: Box::new(guard),
            })
        }),
        pause_hotkeys: Arc::new(move |pause| {
            let mut channel = PipeChannel::new(&resident_pipe_name(), pause_exe.clone());
            let request = if pause {
                Request::PauseHotkeys
            } else {
                Request::ResumeHotkeys
            };
            let _ = forward(&request, &mut channel, WhenNotRunning::GiveUp);
        }),
        program_writable_by_standard_users: program_writable_by_standard_users(&exe),
    };
    run_settings_window(environment)?;
    Ok(())
}

/// 所有界面窗口共用的环境（设置界面、结果面板）。
pub fn ui_environment() -> UiEnvironment {
    let tray = icon::tray_icon();
    UiEnvironment {
        fonts: system_fonts(),
        icon: Some(WindowIcon {
            width: tray.width(),
            height: tray.height(),
            rgba: tray.into_rgba(),
        }),
        system_languages: user_ui_languages(),
        open: Arc::new(|target| open_with_default_program(target).map_err(|e| e.to_string())),
        clipboard: SystemClipboard {
            read_text: Arc::new(|| read_clipboard_text().ok().flatten()),
            write_text: Arc::new(|text| write_clipboard_text(text).map_err(|e| e.to_string())),
        },
    }
}

/// 界面字体：西文用 Segoe UI，中文用微软雅黑（没有时用宋体）。
/// 字体文件只读映射，不复制进进程的私有内存。
fn system_fonts() -> Vec<UiFont> {
    [
        ("Segoe UI", "segoeui.ttf", 0),
        // msyh.ttc 的第二个字体是为界面调过字距的“Microsoft YaHei UI”。
        ("Microsoft YaHei UI", "msyh.ttc", 1),
        ("SimSun", "simsun.ttc", 0),
    ]
    .into_iter()
    .filter_map(|(name, file, index)| {
        map_system_font(file).map(|data| UiFont { name, data, index })
    })
    .collect()
}

/// 普通权限的程序能否改写本程序（程序目录或程序文件本身可写）。
fn program_writable_by_standard_users(exe: &Path) -> bool {
    let dir_writable = exe
        .parent()
        .and_then(standard_user_can_write)
        .unwrap_or(false);
    dir_writable || standard_user_can_write(exe).unwrap_or(false)
}
