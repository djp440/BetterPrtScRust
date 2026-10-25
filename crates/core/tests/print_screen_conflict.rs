//! PrtSc 冲突引导：系统设置把 PrtSc 键交给了系统自带的截图工具时，PrtSc 绑定注册失败，
//! 常驻实例发出引导通知（点击打开对应的系统设置页面）；用户在系统设置里关闭后，
//! PrtSc 绑定立即注册成功。系统设置只读不改（`SystemScreenshotTool` 没有修改的接口）。

use bps_core::{RegistrationError, ResidentInstance};
use bps_platform::{Hotkey, Key, Modifiers, NotificationClick};
use bps_platform_fake::FakePlatform;

const PRINT_SCREEN_AND_SHIFT: &str = r#"
[[bindings]]
hotkey = "PrintScreen"
mode = "display"
after = "copy_to_clipboard"

[[bindings]]
hotkey = "Shift+PrintScreen"
mode = "display"
after = "copy_to_clipboard"
"#;

fn print_screen() -> Hotkey {
    Hotkey::new(Modifiers::NONE, Key::PrintScreen)
}

fn shift_print_screen() -> Hotkey {
    Hotkey::new(Modifiers::SHIFT, Key::PrintScreen)
}

#[test]
fn print_screen_taken_by_system_screenshot_tool_shows_guidance_to_system_settings() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_config_file(PRINT_SCREEN_AND_SHIFT);

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.registered_hotkeys(), [shift_print_screen()]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "PrtSc 键被系统截图工具占用");
    assert!(
        notifications[0]
            .body
            .contains("使用 Print Screen 键打开屏幕截图"),
        "{notifications:?}"
    );
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenPrintScreenKeySettings)
    );
    let failures = instance.registration_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].binding.hotkey, print_screen());
    assert_eq!(
        failures[0].error,
        RegistrationError::TakenBySystemScreenshotTool
    );
    assert_eq!(
        failures[0].error.to_string(),
        "已被系统截图工具占用（可在系统设置里关闭）"
    );
}

#[test]
fn turning_the_system_setting_off_registers_print_screen_binding() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_config_file(PRINT_SCREEN_AND_SHIFT);
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake
        .turn_off_print_screen_key_for_system_screenshot_tool()
        .expect("常驻实例应监视这项系统设置");
    instance.handle(event).unwrap();

    assert_eq!(
        fake.registered_hotkeys(),
        [shift_print_screen(), print_screen()]
    );
    assert!(instance.registration_failures().is_empty());
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 2, "{notifications:?}");
    assert_eq!(notifications[1].title, "PrtSc 键已可用");
}

#[test]
fn print_screen_press_captures_after_the_conflict_is_resolved() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_display(
            bps_platform_fake::FakeDisplay::new(bps_platform::Rect::new(0, 0, 8, 6)).primary(),
        )
        .with_config_file(PRINT_SCREEN_AND_SHIFT);
    let mut instance = ResidentInstance::start(fake.platform());
    assert!(fake.press(print_screen()).is_none());

    let event = fake
        .turn_off_print_screen_key_for_system_screenshot_tool()
        .unwrap();
    instance.handle(event).unwrap();
    instance
        .handle(fake.press(print_screen()).unwrap())
        .unwrap();

    assert!(fake.clipboard().png_image().is_some());
}

#[test]
fn unrelated_keyboard_setting_change_leaves_failure_and_does_not_renotify() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_config_file(PRINT_SCREEN_AND_SHIFT);
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .handle(fake.change_other_keyboard_setting().unwrap())
        .unwrap();

    assert_eq!(fake.registered_hotkeys(), [shift_print_screen()]);
    assert_eq!(instance.registration_failures().len(), 1);
    assert_eq!(fake.notifications().len(), 1, "{:?}", fake.notifications());
}

#[test]
fn print_screen_taken_by_another_program_gets_the_ordinary_failure_notice() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(print_screen())
        .with_config_file(PRINT_SCREEN_AND_SHIFT);

    let instance = ResidentInstance::start(fake.platform());

    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "热键注册失败");
    assert!(matches!(
        instance.registration_failures()[0].error,
        RegistrationError::TakenByOtherProgram
    ));
}

#[test]
fn modified_print_screen_bindings_are_not_affected_by_the_system_setting() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_config_file(&PRINT_SCREEN_AND_SHIFT.replacen(
            "hotkey = \"PrintScreen\"",
            "hotkey = \"Ctrl+PrintScreen\"",
            1,
        ));

    let instance = ResidentInstance::start(fake.platform());

    assert!(instance.registration_failures().is_empty());
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn still_taken_after_setting_change_reports_the_other_program() {
    // 关闭了系统设置，但 PrtSc 仍被别的截图软件占着：改报普通的注册失败。
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_config_file(PRINT_SCREEN_AND_SHIFT);
    let mut instance = ResidentInstance::start(fake.platform());
    let event = fake
        .turn_off_print_screen_key_for_system_screenshot_tool()
        .unwrap();
    let fake = fake.with_hotkey_taken_by_other_program(print_screen());

    instance.handle(event).unwrap();

    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 2, "{notifications:?}");
    assert_eq!(notifications[1].title, "热键注册失败");
    assert!(
        notifications[1]
            .body
            .starts_with("PrintScreen：已被其他程序占用"),
        "{notifications:?}"
    );
    assert!(matches!(
        instance.registration_failures()[0].error,
        RegistrationError::TakenByOtherProgram
    ));
}
