//! 开机自启：常驻实例按配置文件的 `autostart` / `run_as_admin` 登记或移除开机自启，
//! 配置文件修改后立即生效。从 `ResidentInstance::start` 和平台事件驱动，只断言
//! 开机自启登记与通知这些外部可见的结果。

use bps_core::{ResidentInstance, StartOptions};
use bps_platform::{AutostartEntry, AutostartMethod};
use bps_platform_fake::FakePlatform;

use AutostartMethod::{Elevated, Normal};

const BINDING: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
after = "copy_to_clipboard"
"#;

fn config(globals: &str) -> String {
    format!("{globals}\n{BINDING}")
}

fn this_program(method: AutostartMethod) -> AutostartEntry {
    AutostartEntry {
        method,
        this_program: true,
    }
}

fn other_program(method: AutostartMethod) -> AutostartEntry {
    AutostartEntry {
        method,
        this_program: false,
    }
}

#[test]
fn autostart_registers_this_program_to_start_normally_at_logon() {
    let fake = FakePlatform::new().with_config_file(&config("autostart = true"));

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn turning_autostart_off_in_config_removes_registration_immediately() {
    let fake = FakePlatform::new().with_config_file(&config("autostart = true"));
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake.edit_config_file(&config("autostart = false")).unwrap();
    instance.handle(event).unwrap();

    assert!(fake.autostart_entries().is_empty());
}

#[test]
fn turning_autostart_on_in_config_registers_immediately() {
    let fake = FakePlatform::new().with_config_file(&config(""));
    let mut instance = ResidentInstance::start(fake.platform());
    assert!(fake.autostart_entries().is_empty());

    let event = fake.edit_config_file(&config("autostart = true")).unwrap();
    instance.handle(event).unwrap();

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
}

#[test]
fn leftover_registration_of_this_program_is_removed_when_autostart_is_off() {
    let fake = FakePlatform::new()
        .with_autostart_entry(Normal)
        .with_config_file(&config(""));

    let _instance = ResidentInstance::start(fake.platform());

    assert!(fake.autostart_entries().is_empty());
}

#[test]
fn registration_of_another_copy_is_left_alone_when_autostart_is_off() {
    let fake = FakePlatform::new()
        .with_autostart_entry_of_other_program(Normal)
        .with_config_file(&config(""));

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.autostart_entries(), [other_program(Normal)]);
}

#[test]
fn autostart_pointing_elsewhere_is_taken_over_by_this_program() {
    // 例如便携版移动了目录：登记里还是旧位置。
    let fake = FakePlatform::new()
        .with_autostart_entry_of_other_program(Normal)
        .with_config_file(&config("autostart = true"));

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
}

#[test]
fn portable_mode_warns_that_moving_the_program_directory_breaks_autostart() {
    let fake = FakePlatform::new().with_config_file(&config("autostart = true"));

    let _instance = ResidentInstance::start_with(
        fake.platform(),
        StartOptions {
            portable: true,
            ..StartOptions::default()
        },
    );

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "已开启开机自启");
    assert!(
        notifications[0].body.contains("移动程序目录后")
            && notifications[0].body.contains("重新开启"),
        "{notifications:?}"
    );
}

#[test]
fn portable_mode_does_not_warn_again_when_autostart_is_already_registered() {
    let fake = FakePlatform::new()
        .with_autostart_entry(Normal)
        .with_config_file(&config("autostart = true"));

    let _instance = ResidentInstance::start_with(
        fake.platform(),
        StartOptions {
            portable: true,
            ..StartOptions::default()
        },
    );

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn autostart_failure_is_notified_not_silent() {
    let fake = FakePlatform::new()
        .with_autostart_failing("拒绝访问")
        .with_config_file(&config("autostart = true"));

    let _instance = ResidentInstance::start(fake.platform());

    assert!(fake.autostart_entries().is_empty());
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "无法开启开机自启");
    assert!(
        notifications[0].body.contains("拒绝访问"),
        "{notifications:?}"
    );
}

#[test]
fn changing_only_autostart_does_not_reregister_hotkeys_or_renotify_failures() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program("Ctrl+F9".parse().unwrap())
        .with_config_file(&config(""));
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.notifications().len(), 1, "热键注册失败的通知");

    let event = fake.edit_config_file(&config("autostart = true")).unwrap();
    instance.handle(event).unwrap();

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
    assert_eq!(fake.notifications().len(), 1, "{:?}", fake.notifications());
}

#[test]
fn run_as_admin_autostart_uses_elevated_method_instead_of_normal() {
    let fake = FakePlatform::new()
        .as_administrator()
        .with_autostart_entry(Normal)
        .with_config_file(&config("autostart = true\nrun_as_admin = true"));

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.autostart_entries(), [this_program(Elevated)]);
}

#[test]
fn turning_run_as_admin_off_moves_autostart_back_to_normal_method() {
    let fake = FakePlatform::new()
        .as_administrator()
        .with_config_file(&config("autostart = true\nrun_as_admin = true"));
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake.edit_config_file(&config("autostart = true")).unwrap();
    instance.handle(event).unwrap();

    assert_eq!(fake.autostart_entries(), [this_program(Normal)]);
}

#[test]
fn turning_autostart_off_as_administrator_removes_elevated_registration() {
    let fake = FakePlatform::new()
        .as_administrator()
        .with_config_file(&config("autostart = true\nrun_as_admin = true"));
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake
        .edit_config_file(&config("run_as_admin = true"))
        .unwrap();
    instance.handle(event).unwrap();

    assert!(fake.autostart_entries().is_empty());
}
