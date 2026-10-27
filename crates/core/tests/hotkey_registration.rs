//! 热键注册失败不静默：发出托盘通知，失败状态可供设置界面查询。

use bps_core::{RegistrationError, ResidentInstance};
use bps_platform::{Hotkey, Key, Modifiers, NotificationClick};
use bps_platform_fake::FakePlatform;

const F9_AND_F10: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
after = "copy_to_clipboard"

[[bindings]]
hotkey = "Ctrl+F10"
mode = "display"
after = "copy_to_clipboard"
"#;

fn ctrl(key: Key) -> Hotkey {
    Hotkey::new(Modifiers::CTRL, key)
}

#[test]
fn hotkey_taken_by_other_program_is_notified_and_queryable() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(ctrl(Key::F(9)))
        .with_config_file(F9_AND_F10);

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.registered_hotkeys(), [ctrl(Key::F(10))]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "热键注册失败");
    assert!(
        notifications[0].body.starts_with("Ctrl+F9：")
            && notifications[0].body.contains("已被其他程序占用"),
        "{notifications:?}"
    );
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenFile(fake.config_file_path()))
    );

    let failures = instance.registration_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].index, 0);
    assert_eq!(failures[0].binding.hotkey, ctrl(Key::F(9)));
    assert_eq!(failures[0].error, RegistrationError::TakenByOtherProgram);
}

#[test]
fn duplicate_hotkey_in_config_fails_later_binding_only() {
    let fake = FakePlatform::new().with_config_file(&F9_AND_F10.replace("Ctrl+F10", "Control+f9"));

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.registered_hotkeys(), [ctrl(Key::F(9))]);
    let failures = instance.registration_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].index, 1);
    assert_eq!(failures[0].error, RegistrationError::DuplicateOf(0));
    let notifications = fake.notifications();
    assert_eq!(
        notifications[0].body, "Ctrl+F9：与第 1 条热键绑定的热键重复",
        "{notifications:?}"
    );
}

#[test]
fn fixing_config_clears_registration_failures() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(ctrl(Key::F(9)))
        .with_config_file(F9_AND_F10);
    let mut instance = ResidentInstance::start(fake.platform());

    let fixed = F9_AND_F10.replace("Ctrl+F9", "Ctrl+F11");
    instance
        .handle(fake.edit_config_file(&fixed).unwrap())
        .unwrap();

    assert!(instance.registration_failures().is_empty());
    assert_eq!(
        fake.registered_hotkeys(),
        [ctrl(Key::F(11)), ctrl(Key::F(10))]
    );
    assert_eq!(fake.notifications().len(), 1, "修好之后不应再有新的通知");
}
