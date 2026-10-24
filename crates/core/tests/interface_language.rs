//! 界面语言：默认跟随系统，可在配置里指定；切换后常驻实例里的文本（托盘菜单、通知）随之更新。

use bps_core::{Language, Locale, ResidentInstance, Texts};
use bps_platform::{Hotkey, Key, Modifiers};
use bps_platform_fake::FakePlatform;

const ONE_BINDING: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
actions = ["copy_to_clipboard"]
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
