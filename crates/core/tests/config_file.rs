//! 配置文件的生命周期：首次运行写入默认配置、外部修改后自动重新加载、解析失败时保留上一次
//! 有效的配置。从 `ResidentInstance::start` 和平台事件驱动，只断言外部可见的结果。

use std::path::PathBuf;

use bps_core::{CaptureMode, Config, PostCaptureAction, RecordingFormat, ResidentInstance};
use bps_platform::{Hotkey, Key, Modifiers, NotificationClick, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

fn hotkey(modifiers: Modifiers, key: Key) -> Hotkey {
    Hotkey::new(modifiers, key)
}

/// spec #1 的默认热键表。
fn default_hotkeys() -> Vec<Hotkey> {
    vec![
        hotkey(Modifiers::NONE, Key::PrintScreen),
        hotkey(Modifiers::SHIFT, Key::PrintScreen),
        hotkey(Modifiers::CTRL, Key::PrintScreen),
        hotkey(Modifiers::ALT, Key::PrintScreen),
        hotkey(Modifiers::CTRL.with(Modifiers::SHIFT), Key::PrintScreen),
        hotkey(Modifiers::WIN.with(Modifiers::SHIFT), Key::Letter('O')),
    ]
}

#[test]
fn first_run_writes_config_file_with_six_default_bindings() {
    use CaptureMode::*;
    use PostCaptureAction::*;
    let fake = FakePlatform::new();

    let _instance = ResidentInstance::start(fake.platform());

    let text = fake.config_file_text().expect("首次运行应写入配置文件");
    let config = Config::parse(&text).unwrap();
    let bindings: Vec<_> = config
        .bindings
        .iter()
        .map(|b| (b.hotkey, b.capture.mode, b.capture.actions.clone()))
        .collect();
    let hotkeys = default_hotkeys();
    assert_eq!(
        bindings,
        [
            (hotkeys[0], Region, vec![Annotate]),
            (hotkeys[1], Display, vec![CopyToClipboard, AutoSave]),
            (hotkeys[2], Window, vec![CopyToClipboard, AutoSave]),
            (hotkeys[3], Region, vec![Record]),
            (hotkeys[4], Region, vec![LongScreenshot]),
            (hotkeys[5], Region, vec![Ocr]),
        ]
    );
    assert_eq!(
        config.bindings[3].capture.recording.format,
        Some(RecordingFormat::Mp4),
        "Alt+PrtSc 默认录制 MP4"
    );
}

#[test]
fn first_run_registers_hotkeys_of_default_bindings_without_notifications() {
    let fake = FakePlatform::new();

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.registered_hotkeys(), default_hotkeys());
    assert!(instance.registration_failures().is_empty());
    assert_eq!(fake.notifications(), []);
}

const ONLY_F9_COPIES_DISPLAY: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
actions = ["copy_to_clipboard"]
"#;

#[test]
fn existing_config_file_is_used_and_left_untouched() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_config_file(ONLY_F9_COPIES_DISPLAY);

    let mut instance = ResidentInstance::start(fake.platform());

    assert_eq!(
        fake.config_file_text().as_deref(),
        Some(ONLY_F9_COPIES_DISPLAY)
    );
    let ctrl_f9 = hotkey(Modifiers::CTRL, Key::F(9));
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9]);
    instance.handle(fake.press(ctrl_f9).unwrap()).unwrap();
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

const GLOBAL_JPEG_TO_CUSTOM_DIR: &str = r#"
save_dir = 'D:\截图'
output_format = "jpg"

[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
actions = ["copy_to_clipboard", "auto_save"]
"#;

#[test]
fn global_output_settings_apply_to_hotkey_bindings() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_config_file(GLOBAL_JPEG_TO_CUSTOM_DIR);
    let mut instance = ResidentInstance::start(fake.platform());

    let ctrl_f9 = hotkey(Modifiers::CTRL, Key::F(9));
    instance.handle(fake.press(ctrl_f9).unwrap()).unwrap();

    // 默认命名模板 + 假时钟的默认时间。
    let saved = PathBuf::from(r"D:\截图\2000-01-01 00-00-00.jpg");
    assert_eq!(fake.files(), std::slice::from_ref(&saved));
    assert_eq!(fake.clipboard().formats(), ["JPEG", "DIB", "Files"]);
}

const ONLY_F10_COPIES_DISPLAY: &str = r#"
[[bindings]]
hotkey = "Ctrl+F10"
mode = "display"
actions = ["copy_to_clipboard"]
"#;

#[test]
fn editing_config_file_rebinds_hotkeys_without_restart() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_config_file(ONLY_F9_COPIES_DISPLAY);
    let mut instance = ResidentInstance::start(fake.platform());

    let event = fake
        .edit_config_file(ONLY_F10_COPIES_DISPLAY)
        .expect("常驻实例应监视配置文件");
    instance.handle(event).unwrap();

    let ctrl_f10 = hotkey(Modifiers::CTRL, Key::F(10));
    assert_eq!(fake.registered_hotkeys(), [ctrl_f10]);
    instance.handle(fake.press(ctrl_f10).unwrap()).unwrap();
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
    assert_eq!(fake.notifications(), []);
}

#[test]
fn invalid_config_edit_keeps_previous_config_and_notifies_with_error_line() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_config_file(ONLY_F9_COPIES_DISPLAY);
    let mut instance = ResidentInstance::start(fake.platform());

    // 第 4 行的捕获模式拼错了。
    let broken = ONLY_F10_COPIES_DISPLAY.replace("\"display\"", "\"dispaly\"");
    instance
        .handle(fake.edit_config_file(&broken).unwrap())
        .unwrap();

    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    let notification = &notifications[0];
    assert_eq!(notification.title, "配置文件有错误");
    assert!(
        notification.body.contains("继续使用上一次有效的配置")
            && notification.body.contains("第 4 行"),
        "{notification:?}"
    );
    assert_eq!(
        notification.on_click,
        Some(NotificationClick::OpenFile(fake.config_file_path()))
    );
    let ctrl_f9 = hotkey(Modifiers::CTRL, Key::F(9));
    assert_eq!(fake.registered_hotkeys(), [ctrl_f9]);
    instance.handle(fake.press(ctrl_f9).unwrap()).unwrap();
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

#[test]
fn saving_config_file_without_changes_does_not_repeat_notifications() {
    let ctrl_f9 = hotkey(Modifiers::CTRL, Key::F(9));
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(ctrl_f9)
        .with_config_file(ONLY_F9_COPIES_DISPLAY);
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(
        fake.notifications().len(),
        1,
        "启动时应通知 Ctrl+F9 注册失败"
    );

    instance
        .handle(fake.edit_config_file(ONLY_F9_COPIES_DISPLAY).unwrap())
        .unwrap();

    assert_eq!(fake.notifications().len(), 1);
}

#[test]
fn deleting_config_file_keeps_current_config_and_notifies() {
    let fake = FakePlatform::new().with_config_file(ONLY_F9_COPIES_DISPLAY);
    let mut instance = ResidentInstance::start(fake.platform());

    instance.handle(fake.delete_config_file().unwrap()).unwrap();

    assert_eq!(
        fake.registered_hotkeys(),
        [hotkey(Modifiers::CTRL, Key::F(9))]
    );
    let titles: Vec<_> = fake.notifications().into_iter().map(|n| n.title).collect();
    assert_eq!(titles, ["配置文件已被删除"]);
}

#[test]
fn broken_config_file_at_startup_uses_default_bindings_and_is_not_overwritten() {
    let broken = "[[bindings]]\nhotkey = \"Ctrl+F9\"\n";
    let fake = FakePlatform::new().with_config_file(broken);

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.registered_hotkeys(), default_hotkeys());
    assert_eq!(instance.config(), &Config::default());
    assert_eq!(fake.config_file_text().as_deref(), Some(broken));
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "配置文件有错误");
    assert!(
        notifications[0].body.contains("暂时使用默认配置"),
        "{notifications:?}"
    );
}
