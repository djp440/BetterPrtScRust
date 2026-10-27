//! 以管理员身份运行：配置开启 `run_as_admin` 而常驻实例没有管理员权限时，以管理员身份
//! 重新启动，由新实例接替；用户拒绝时继续以普通权限运行并说明后果，且不反复请求。

use bps_core::{Flow, ResidentInstance, StartOptions};
use bps_platform::{AutostartEntry, AutostartMethod, Hotkey};
use bps_platform_fake::FakePlatform;

const BINDING: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
after = "copy_to_clipboard"
"#;

fn config(globals: &str) -> String {
    format!("{globals}\n{BINDING}")
}

fn ctrl_f9() -> Hotkey {
    "Ctrl+F9".parse().unwrap()
}

#[test]
fn run_as_admin_without_admin_rights_hands_over_to_an_elevated_instance() {
    let fake = FakePlatform::new().with_config_file(&config("run_as_admin = true"));

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.administrator_restarts(), 1);
    assert!(instance.restarting_as_administrator());
    assert!(
        fake.registered_hotkeys().is_empty(),
        "让出单实例前不应占用热键"
    );
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn elevated_instance_runs_normally_without_restarting() {
    let fake = FakePlatform::new()
        .as_administrator()
        .with_config_file(&config("run_as_admin = true"));

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.administrator_restarts(), 0);
    assert!(!instance.restarting_as_administrator());
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9()]);
}

#[test]
fn declined_uac_prompt_keeps_running_with_normal_rights_and_explains_why() {
    let fake = FakePlatform::new()
        .with_administrator_restart_declined()
        .with_config_file(&config("run_as_admin = true"));

    let instance = ResidentInstance::start(fake.platform());

    assert!(!instance.restarting_as_administrator());
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9()]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "未能以管理员身份运行");
    assert!(
        notifications[0].body.contains("管理员权限的窗口在前台时"),
        "{notifications:?}"
    );
}

#[test]
fn declined_uac_prompt_keeps_existing_normal_autostart_until_elevated() {
    let fake = FakePlatform::new()
        .with_administrator_restart_declined()
        .with_autostart_entry(AutostartMethod::Normal)
        .with_config_file(&config("autostart = true\nrun_as_admin = true"));

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(
        fake.autostart_entries(),
        [AutostartEntry {
            method: AutostartMethod::Normal,
            this_program: true,
        }]
    );
    assert_eq!(fake.notifications().len(), 1, "{:?}", fake.notifications());
}

#[test]
fn restarted_instance_still_without_admin_rights_does_not_restart_again() {
    let fake = FakePlatform::new().with_config_file(&config("run_as_admin = true"));

    let instance = ResidentInstance::start_with(
        fake.platform(),
        StartOptions {
            restarted_as_administrator: true,
            ..StartOptions::default()
        },
    );

    assert_eq!(fake.administrator_restarts(), 0);
    assert!(!instance.restarting_as_administrator());
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9()]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "未能以管理员身份运行");
}

#[test]
fn turning_run_as_admin_on_in_config_hands_over_to_an_elevated_instance() {
    let fake = FakePlatform::new().with_config_file(&config(""));
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9()]);

    let event = fake
        .edit_config_file(&config("run_as_admin = true"))
        .unwrap();
    let flow = instance.handle(event).unwrap();

    assert_eq!(flow, Flow::Exit);
    assert_eq!(fake.administrator_restarts(), 1);
    assert!(fake.registered_hotkeys().is_empty(), "退出前应注销热键");
}

#[test]
fn declined_uac_prompt_is_not_repeated_on_unrelated_config_changes() {
    let fake = FakePlatform::new()
        .with_administrator_restart_declined()
        .with_config_file(&config(""));
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake
        .edit_config_file(&config("run_as_admin = true"))
        .unwrap();
    assert_eq!(instance.handle(event).unwrap(), Flow::Continue);
    let event = fake
        .edit_config_file(&config("run_as_admin = true\nautostart = true"))
        .unwrap();
    assert_eq!(instance.handle(event).unwrap(), Flow::Continue);

    assert_eq!(fake.administrator_restarts(), 1);
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9()]);
}
