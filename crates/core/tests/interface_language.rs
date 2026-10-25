//! 界面语言：默认跟随系统，可在配置里指定；切换后常驻实例里的文本（托盘菜单、通知、选区遮罩上的提示、标注工具条）
//! 随之更新。

use bps_core::{
    CaptureMode, CaptureRequest, Command, Language, Locale, PostCaptureAction, ResidentInstance,
    Texts,
};
use bps_platform::{Hotkey, Key, Modifiers, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

const ONE_BINDING: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
after = "copy_to_clipboard"
"#;

fn with_language(language: &str) -> String {
    format!("language = \"{language}\"\n{ONE_BINDING}")
}

#[test]
fn tray_menu_follows_chinese_system_language_by_default() {
    let fake = FakePlatform::new()
        .with_system_languages(&["zh-CN", "en-US"])
        .with_config_file(ONE_BINDING);

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.tray_menu_labels(), ["设置", "退出"]);
}

#[test]
fn tray_menu_follows_english_system_language_by_default() {
    let fake = FakePlatform::new()
        .with_system_languages(&["en-GB"])
        .with_config_file(ONE_BINDING);

    let _instance = ResidentInstance::start(fake.platform());

    assert_eq!(fake.tray_menu_labels(), ["Settings", "Exit"]);
}

#[test]
fn unsupported_system_language_falls_back_to_the_next_preference_then_english() {
    let fake = FakePlatform::new().with_system_languages(&["ja-JP", "zh-Hans-CN"]);
    let _instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.tray_menu_labels(), ["设置", "退出"]);

    let fake = FakePlatform::new().with_system_languages(&["ja-JP"]);
    let _instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.tray_menu_labels(), ["Settings", "Exit"]);
}

#[test]
fn configured_language_overrides_system_language() {
    let fake = FakePlatform::new()
        .with_system_languages(&["zh-CN"])
        .with_config_file(&with_language("en"));

    let instance = ResidentInstance::start(fake.platform());

    assert_eq!(instance.config().language, Language::En);
    assert_eq!(fake.tray_menu_labels(), ["Settings", "Exit"]);
}

#[test]
fn switching_language_updates_tray_menu_and_later_notifications() {
    let taken = Hotkey::new(Modifiers::CTRL, Key::F(9));
    let fake = FakePlatform::new()
        .with_system_languages(&["zh-CN"])
        .with_hotkey_taken_by_other_program(taken)
        .with_config_file(ONE_BINDING);
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.notifications()[0].title, "热键注册失败");

    let event = fake
        .edit_config_file(&with_language("en").replace("Ctrl+F9", "Ctrl+F8"))
        .unwrap();
    instance.handle(event).unwrap();
    assert_eq!(fake.tray_menu_labels(), ["Settings", "Exit"]);

    let event = fake.edit_config_file(&with_language("en")).unwrap();
    instance.handle(event).unwrap();
    let last = fake.notifications().pop().unwrap();
    assert_eq!(last.title, "Hotkey registration failed");
    assert_eq!(last.body, "Ctrl+F9: Already in use by another program");

    let event = fake.edit_config_file(&with_language("zh-CN")).unwrap();
    instance.handle(event).unwrap();
    assert_eq!(fake.tray_menu_labels(), ["设置", "退出"]);
}

#[test]
fn changing_only_the_language_does_not_reregister_hotkeys_or_repeat_failure_notices() {
    let taken = Hotkey::new(Modifiers::CTRL, Key::F(9));
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(taken)
        .with_config_file(ONE_BINDING);
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(fake.notifications().len(), 1);

    let event = fake.edit_config_file(&with_language("en")).unwrap();
    instance.handle(event).unwrap();

    assert_eq!(fake.notifications().len(), 1, "{:?}", fake.notifications());
    assert_eq!(instance.registration_failures().len(), 1);
}

#[test]
fn unknown_language_in_config_is_reported_with_line() {
    let fake = FakePlatform::new().with_config_file(&with_language("fr"));

    let _instance = ResidentInstance::start(fake.platform());

    let notification = &fake.notifications()[0];
    assert_eq!(notification.title, "配置文件有错误");
    assert!(
        notification.body.contains("第 1 行") && notification.body.contains("“fr”"),
        "{notification:?}"
    );
}

#[test]
fn both_languages_provide_the_same_texts() {
    let zh = Texts::of(Locale::ZhCn);
    let en = Texts::of(Locale::En);

    let mut zh_ids = zh.message_ids();
    let mut en_ids = en.message_ids();
    zh_ids.sort_unstable();
    en_ids.sort_unstable();

    assert!(!zh_ids.is_empty());
    assert_eq!(zh_ids, en_ids);
    assert_eq!(zh.get("tray-exit"), "退出");
    assert_eq!(en.get("tray-exit"), "Exit");
}

#[test]
fn language_names_in_config_round_trip() {
    for language in [Language::Auto, Language::ZhCn, Language::En] {
        assert_eq!(language.config_name().parse::<Language>(), Ok(language));
    }
    assert_eq!("zh-cn".parse::<Language>(), Ok(Language::ZhCn));
    assert!("fr".parse::<Language>().is_err());
}

#[test]
fn print_screen_and_administrator_notices_follow_the_interface_language() {
    let fake = FakePlatform::new()
        .with_print_screen_key_taken_by_system_screenshot_tool()
        .with_administrator_restart_declined()
        .with_config_file(
            "language = \"en\"\nrun_as_admin = true\n\n[[bindings]]\nhotkey = \"PrintScreen\"\nmode = \"region\"\nafter = \"copy_to_clipboard\"\n",
        );

    let _instance = ResidentInstance::start(fake.platform());

    let titles: Vec<String> = fake.notifications().into_iter().map(|n| n.title).collect();
    assert_eq!(
        titles,
        [
            "Could not run as administrator",
            "The PrtSc key is taken by the Windows screenshot tool",
        ]
    );
}

#[test]
fn selection_overlay_hint_follows_the_interface_language() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 200, 150)).primary())
        .with_system_languages(&["zh-CN"])
        .with_config_file(&with_language("en"));
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::CopyToClipboard,
        )))
        .unwrap();
    for event in fake.drag(Point::new(20, 40), Point::new(119, 89)) {
        instance.handle(event).unwrap();
    }

    let labels: Vec<String> = fake
        .overlay_scene()
        .unwrap()
        .labels()
        .map(|label| label.text.clone())
        .collect();
    assert_eq!(
        labels,
        [
            "100 × 50",
            "Enter or double-click the selection: done   Esc: cancel"
        ]
    );
}

#[test]
fn annotation_toolbar_follows_the_interface_language() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_system_languages(&["zh-CN"])
        .with_config_file(&with_language("en"));
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    for event in fake.drag(Point::new(20, 40), Point::new(119, 89)) {
        instance.handle(event).unwrap();
    }

    let scene = fake.overlay_scene().unwrap();
    for text in [
        "Rectangle",
        "Arrow",
        "Record",
        "Long shot",
        "OCR",
        "Translate",
        "Cancel",
        "Save as",
        "Confirm",
    ] {
        assert!(scene.text_rect(text).is_some(), "工具条上应有“{text}”");
    }
    assert!(scene.text_rect("复制").is_none());
    for event in fake.click_text("Confirm") {
        instance.handle(event).unwrap();
    }
    assert!(!fake.overlay_visible());
    let image = fake.clipboard().png_image().expect("应复制了截图");
    assert_eq!((image.width(), image.height()), (100, 50));
}

#[test]
fn toml_syntax_error_in_config_is_explained_in_the_interface_language() {
    for (system_language, prefix) in [
        ("zh-CN", "配置文件格式错误："),
        ("en-US", "Malformed config file: "),
    ] {
        let fake = FakePlatform::new()
            .with_system_languages(&[system_language])
            .with_config_file("hotkey = = 3\n");

        let _instance = ResidentInstance::start(fake.platform());

        let notifications = fake.notifications();
        let body = &notifications[0].body;
        assert!(body.contains(prefix), "{system_language}: {body}");
    }
}
