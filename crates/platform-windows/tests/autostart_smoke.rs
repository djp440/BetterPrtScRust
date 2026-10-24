//! 真机冒烟：开机自启（注册表 Run 项、计划任务）、进程权限、PrtSc 键的系统设置。默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows --test autostart_smoke -- --ignored --test-threads=1`
//!
//! 只用带进程号的测试专用名称登记，指向一个不存在的程序，结束时（包括断言失败时）删除；
//! 不读写真实的 `BetterPrtSc` 登记。系统设置只读不改。
#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use bps_platform::{
    Autostart, AutostartEntry, AutostartMethod, ConfigFile, PlatformEvent, SystemScreenshotTool,
};
use bps_platform_windows::{EventLoop, WindowsAutostart, is_elevated};

/// 测试专用的登记名称和一个不存在的程序路径：即使清理失败，登录时也不会真的启动什么。
fn smoke_autostart(highest: bool) -> WindowsAutostart {
    let id = std::process::id();
    WindowsAutostart::for_smoke_test(
        &format!("BetterPrtSc-SmokeTest-{id}"),
        &format!("BetterPrtSc SmokeTest {id}"),
        smoke_program(),
        highest,
    )
}

fn smoke_program() -> PathBuf {
    std::env::temp_dir()
        .join(format!("bps-autostart-smoke-{}", std::process::id()))
        .join("不存在的 BetterPrtSc.exe")
}

/// 结束时（包括 panic 时）移除测试登记。
struct Cleanup(bool);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let mut autostart = smoke_autostart(self.0);
        let _ = autostart.disable(AutostartMethod::Normal);
        let _ = autostart.disable(AutostartMethod::Elevated);
    }
}

fn run_value(name: &str) -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            name,
        ])
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
#[ignore = "真机冒烟：写入并删除测试专用的 HKCU Run 项"]
fn run_key_autostart_is_written_read_back_and_removed() {
    let _cleanup = Cleanup(false);
    let mut autostart = smoke_autostart(false);
    let value_name = format!("BetterPrtSc-SmokeTest-{}", std::process::id());
    assert_eq!(autostart.entries().unwrap(), []);

    autostart.enable(AutostartMethod::Normal).unwrap();
    let written = run_value(&value_name);
    let entries = autostart.entries().unwrap();
    autostart.disable(AutostartMethod::Normal).unwrap();
    autostart.disable(AutostartMethod::Normal).unwrap(); // 已经没有时不报错

    assert_eq!(
        entries,
        [AutostartEntry {
            method: AutostartMethod::Normal,
            this_program: true,
        }]
    );
    // reg 的输出按控制台代码页编码，只核对 ASCII 部分：路径带引号、以 .exe" 结尾。
    let written = written.expect("应写入 Run 项");
    let dir = smoke_program().parent().unwrap().display().to_string();
    assert!(
        written.contains(&format!("\"{dir}\\")) && written.trim_end().ends_with(".exe\""),
        "程序路径应带引号：{written}"
    );
    assert_eq!(run_value(&value_name), None, "应已删除 Run 项");
    assert_eq!(autostart.entries().unwrap(), []);
}

#[test]
#[ignore = "真机冒烟：登记、读回并删除一个以普通权限运行的测试计划任务"]
fn logon_task_is_registered_read_back_and_deleted() {
    let _cleanup = Cleanup(false);
    // 以普通权限运行的任务不需要管理员权限即可登记，用它验证计划任务的登记、读取与删除。
    let mut autostart = smoke_autostart(false);

    autostart.enable(AutostartMethod::Elevated).unwrap();
    let entries = autostart.entries().unwrap();
    autostart.disable(AutostartMethod::Elevated).unwrap();
    autostart.disable(AutostartMethod::Elevated).unwrap(); // 已经没有时不报错

    assert_eq!(
        entries,
        [AutostartEntry {
            method: AutostartMethod::Elevated,
            this_program: true,
        }]
    );
    assert_eq!(autostart.entries().unwrap(), []);
}

#[test]
#[ignore = "真机冒烟：不提升权限时登记最高权限的计划任务应被拒绝"]
fn highest_privilege_task_requires_admin_rights() {
    let _cleanup = Cleanup(true);
    eprintln!("当前进程以管理员权限运行：{}", is_elevated());
    if is_elevated() {
        eprintln!("已是管理员，跳过");
        return;
    }
    let mut autostart = smoke_autostart(true);

    let result = autostart.enable(AutostartMethod::Elevated);

    let error = result.expect_err("没有管理员权限时应失败");
    eprintln!("{error}");
    assert!(error.message().contains("需要管理员权限"), "{error}");
    assert_eq!(autostart.entries().unwrap(), []);
}

#[test]
#[ignore = "真机冒烟：只读 PrtSc 键的系统设置，并与配置文件一起监视"]
fn print_screen_key_setting_is_read_and_watched_alongside_config_file() {
    let dir = std::env::temp_dir().join(format!("bps-keyboard-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let event_loop = EventLoop::new().unwrap();
    let mut tool = event_loop.system_screenshot_tool();
    let mut config = event_loop.config_file(dir.join("config.toml"));

    let takes = tool.takes_print_screen_key().unwrap();
    eprintln!("系统截图工具占用 PrtSc：{takes}");
    tool.watch().unwrap();
    config.write("# 第一版\n").unwrap();
    config.watch().unwrap();
    let idle = event_loop.wait_event(Duration::from_millis(600));
    // 两个监视句柄同时等待时，配置文件变化仍应递交为配置文件事件。
    fs::write(dir.join("config.toml"), "# 第二版\n").unwrap();
    let event = event_loop.wait_event(Duration::from_secs(3));
    drop(config);
    drop(tool);
    drop(event_loop);
    fs::remove_dir_all(&dir).unwrap();

    assert!(idle.is_none(), "没有变化时不应有事件：{idle:?}");
    assert!(
        matches!(event, Some(PlatformEvent::ConfigFileChanged)),
        "{event:?}"
    );
}
