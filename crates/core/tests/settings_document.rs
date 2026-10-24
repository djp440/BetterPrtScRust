//! 设置界面的数据模型：读取 config.toml → 修改 → 写回文本。写回的文本保留用户的注释和
//! 没在界面上出现的覆盖项；常驻实例重新加载写回的文件后立即生效。

use std::path::PathBuf;

use bps_core::{
    BindingSettings, CaptureMode, Config, DEFAULT_CONFIG, DEFAULT_FILENAME_TEMPLATE, Language,
    Locale, OutputFormat, PostCaptureAction, RecordingFormat, ResidentInstance, SaveDir,
    SettingsDocument, SettingsProblem, Texts,
};
use bps_platform::{Hotkey, Key, Modifiers};
use bps_platform_fake::FakePlatform;

fn hotkey(text: &str) -> Hotkey {
    text.parse().unwrap()
}

fn hotkeys(config: &Config) -> Vec<String> {
    config
        .bindings
        .iter()
        .map(|b| b.hotkey.to_string())
        .collect()
}

#[test]
fn default_config_exposes_every_setting_with_its_default() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let settings = document.settings();

    assert_eq!(settings.language, Language::Auto);
    assert!(!settings.check_for_updates);
    assert!(!settings.autostart);
    assert!(!settings.run_as_admin);
    assert_eq!(settings.save_dir, None);
    assert_eq!(settings.filename_template, DEFAULT_FILENAME_TEMPLATE);
    assert_eq!(settings.output_format, OutputFormat::Png);
    let listed: Vec<String> = settings
        .bindings
        .iter()
        .map(|b| b.hotkey.to_string())
        .collect();
    assert_eq!(
        listed,
        [
            "PrintScreen",
            "Shift+PrintScreen",
            "Ctrl+PrintScreen",
            "Alt+PrintScreen",
            "Ctrl+Shift+PrintScreen",
            "Win+Shift+O",
        ]
    );
    assert_eq!(settings.bindings[1].mode, CaptureMode::Display);
    assert_eq!(
        settings.bindings[1].actions,
        [
            PostCaptureAction::CopyToClipboard,
            PostCaptureAction::AutoSave
        ]
    );
}

#[test]
fn unchanged_settings_write_back_the_file_byte_for_byte() {
    let commented = DEFAULT_CONFIG.replace("mode = \"region\"", "mode = \"region\"  # 区域");
    let text = format!("\u{feff}# 我的配置\r\n{commented}");
    let document = SettingsDocument::parse(&text).unwrap();

    assert_eq!(document.render(document.settings()).unwrap(), text);
}

#[test]
fn global_settings_are_written_back_and_keep_the_comments() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    settings.language = Language::En;
    settings.check_for_updates = true;
    settings.autostart = true;
    settings.run_as_admin = true;
    settings.save_dir = Some(PathBuf::from(r"D:\截图"));
    settings.filename_template = "{mode} {yyyy-MM-dd}".to_owned();
    settings.output_format = OutputFormat::Jpeg;

    let text = document.render(&settings).unwrap();

    let config = Config::parse(&text).unwrap();
    assert_eq!(config.language, Language::En);
    assert!(config.check_for_updates && config.autostart && config.run_as_admin);
    assert_eq!(config.output.save_dir, SaveDir::Custom(r"D:\截图".into()));
    assert_eq!(
        config.output.filename_template.as_str(),
        "{mode} {yyyy-MM-dd}"
    );
    assert_eq!(config.output.output_format, OutputFormat::Jpeg);
    assert_eq!(hotkeys(&config), hotkeys(&Config::default()));
    assert_eq!(
        config.bindings[1].capture.output.output_format,
        OutputFormat::Jpeg,
        "没有覆盖项的绑定跟随全局设置"
    );
    assert_eq!(
        config.bindings[3].capture.recording.format,
        Some(RecordingFormat::Mp4)
    );
    // 文件开头的说明注释仍在最前面，新写入的全局设置在它之后、第一条热键绑定之前。
    assert!(text.starts_with("# BetterPrtSc 配置文件\n"), "{text}");
    let language_at = text.find("language = \"en\"").unwrap();
    let last_header_comment = text.find("#   frame_rate").unwrap();
    let first_binding = text.find("\n[[bindings]]\n").unwrap();
    assert!(
        last_header_comment < language_at && language_at < first_binding,
        "{text}"
    );
    assert!(text.contains(r"save_dir = 'D:\截图'"), "{text}");
}

#[test]
fn choosing_the_default_save_dir_again_removes_the_setting() {
    let text = "# 我的存储目录\nsave_dir = 'D:\\截图'\noutput_format = \"webp\"\n";
    let document = SettingsDocument::parse(text).unwrap();
    let mut settings = document.settings().clone();
    assert_eq!(settings.save_dir, Some(PathBuf::from(r"D:\截图")));
    settings.save_dir = None;

    let written = document.render(&settings).unwrap();

    assert_eq!(written, "# 我的存储目录\noutput_format = \"webp\"\n");
}

#[test]
fn bindings_can_be_added_modified_and_removed() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    settings.bindings.remove(0); // PrintScreen
    let shift_prtsc = &mut settings.bindings[0];
    shift_prtsc.hotkey = hotkey("Ctrl+Alt+S");
    shift_prtsc.mode = CaptureMode::VirtualDesktop;
    shift_prtsc.actions = vec![PostCaptureAction::CopyToClipboard];
    settings.bindings.push(BindingSettings::new(
        hotkey("Ctrl+F7"),
        CaptureMode::Display,
        vec![PostCaptureAction::AutoSave],
    ));

    let text = document.render(&settings).unwrap();

    let config = Config::parse(&text).unwrap();
    assert_eq!(
        hotkeys(&config),
        [
            "Ctrl+Alt+S",
            "Ctrl+PrintScreen",
            "Alt+PrintScreen",
            "Ctrl+Shift+PrintScreen",
            "Win+Shift+O",
            "Ctrl+F7",
        ]
    );
    assert_eq!(config.bindings[0].capture.mode, CaptureMode::VirtualDesktop);
    assert_eq!(
        config.bindings[0].capture.actions,
        [PostCaptureAction::CopyToClipboard]
    );
    // 界面上没有的覆盖项（录屏绑定的 mp4）保留下来。
    assert_eq!(
        config.bindings[2].capture.recording.format,
        Some(RecordingFormat::Mp4)
    );
    assert_eq!(config.bindings[5].capture.mode, CaptureMode::Display);
    assert_eq!(
        config.bindings[5].capture.actions,
        [PostCaptureAction::AutoSave]
    );
    assert!(text.starts_with("# BetterPrtSc 配置文件\n"), "{text}");
    assert!(
        text.contains("\n\n[[bindings]]\nhotkey = \"Ctrl+F7\""),
        "{text}"
    );
}

#[test]
fn comments_and_overrides_of_untouched_bindings_survive_edits() {
    let text = r#"[[bindings]]
hotkey = "Ctrl+F8"
mode = "display"
actions = ["copy_to_clipboard"]

# 存到 D 盘的那条
[[bindings]]
hotkey = "Ctrl+F9"  # 别改
mode = "region"
actions = ["auto_save"]
save_dir = 'D:\Shots'
frame_rate = 30
"#;
    let document = SettingsDocument::parse(text).unwrap();
    let mut settings = document.settings().clone();
    settings.bindings[0]
        .actions
        .push(PostCaptureAction::AutoSave);

    let written = document.render(&settings).unwrap();

    assert!(
        written.ends_with(
            r#"
# 存到 D 盘的那条
[[bindings]]
hotkey = "Ctrl+F9"  # 别改
mode = "region"
actions = ["auto_save"]
save_dir = 'D:\Shots'
frame_rate = 30
"#
        ),
        "{written}"
    );
    assert!(
        written.contains(r#"actions = ["copy_to_clipboard", "auto_save"]"#),
        "{written}"
    );
}

#[test]
fn removing_the_first_binding_keeps_the_file_header() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    settings.bindings.clear();

    let text = document.render(&settings).unwrap();

    assert!(text.starts_with("# BetterPrtSc 配置文件\n"), "{text}");
    assert!(text.contains("#   frame_rate"), "{text}");
    assert!(Config::parse(&text).unwrap().bindings.is_empty());
}

#[test]
fn changing_actions_drops_an_output_format_that_no_longer_fits() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    // Alt+PrtSc 原来是录屏（output_format = "mp4"），改成截图。
    settings.bindings[3].actions = vec![PostCaptureAction::CopyToClipboard];
    settings.output_format = OutputFormat::WebP;

    let config = Config::parse(&document.render(&settings).unwrap()).unwrap();

    assert_eq!(
        config.bindings[3].capture.output.output_format,
        OutputFormat::WebP
    );
    assert_eq!(config.bindings[3].capture.recording.format, None);
}

#[test]
fn invalid_settings_are_rejected_with_reasons_in_the_interface_language() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    settings.filename_template = "{title}".to_owned();
    settings.save_dir = Some(PathBuf::from("  "));
    settings.bindings[1].hotkey = Hotkey::new(Modifiers::SHIFT, Key::Letter('A'));
    settings.bindings[2].actions.clear();

    let problems = document.render(&settings).unwrap_err();

    assert_eq!(problems.len(), 4, "{problems:?}");
    assert!(problems.contains(&SettingsProblem::EmptySaveDir));
    assert!(
        problems.contains(&SettingsProblem::TypingKeyWithoutModifier {
            binding: 1,
            hotkey: Hotkey::new(Modifiers::SHIFT, Key::Letter('A')),
        })
    );
    assert!(problems.contains(&SettingsProblem::NoActions { binding: 2 }));
    let en = Texts::of(Locale::En);
    let zh = Texts::of(Locale::ZhCn);
    let english: Vec<String> = problems.iter().map(|p| p.localized(en)).collect();
    let chinese: Vec<String> = problems.iter().map(|p| p.localized(zh)).collect();
    assert!(english.iter().any(|m| m.contains("{title}")), "{english:?}");
    assert!(
        english.iter().any(|m| m.contains("Binding 2")
            && m.contains("Shift+A")
            && m.contains("Ctrl, Alt or Win")),
        "{english:?}"
    );
    assert!(
        chinese.iter().any(|m| m.contains("第 3 条热键绑定")),
        "{chinese:?}"
    );
}

#[test]
fn problems_are_known_while_editing_before_saving() {
    let document = SettingsDocument::parse(DEFAULT_CONFIG).unwrap();
    let mut settings = document.settings().clone();
    assert_eq!(settings.problems(), []);

    settings.bindings.push(BindingSettings::new(
        hotkey("Ctrl+F9"),
        CaptureMode::Region,
        Vec::new(),
    ));

    assert_eq!(
        settings.problems(),
        [SettingsProblem::NoActions { binding: 6 }]
    );
    assert_eq!(settings.problems()[0].binding(), Some(6));
}

#[test]
fn broken_config_cannot_be_edited_and_says_where_it_is_broken() {
    let error = SettingsDocument::parse(
        "[[bindings]]\nhotkey = \"Ctrl+F9\"\nmode = \"nope\"\nactions = [\"ocr\"]\n",
    )
    .unwrap_err();

    assert_eq!(error.line(), Some(3));
}

#[test]
fn saved_settings_take_effect_when_the_resident_instance_reloads_the_file() {
    let fake = FakePlatform::new().with_system_languages(&["zh-CN"]);
    let mut instance = ResidentInstance::start(fake.platform());
    let on_disk = fake.config_file_text().unwrap();
    let document = SettingsDocument::parse(&on_disk).unwrap();
    let mut settings = document.settings().clone();
    settings.language = Language::En;
    settings
        .bindings
        .retain(|b| b.hotkey != hotkey("PrintScreen"));
    settings.bindings.push(BindingSettings::new(
        hotkey("Ctrl+F7"),
        CaptureMode::Display,
        vec![PostCaptureAction::CopyToClipboard],
    ));

    let text = document.render(&settings).unwrap();
    let changed = fake.edit_config_file(&text).unwrap();
    instance.handle(changed).unwrap();

    assert_eq!(fake.tray_menu_labels(), ["Settings", "Exit"]);
    assert!(fake.registered_hotkeys().contains(&hotkey("Ctrl+F7")));
    assert!(!fake.registered_hotkeys().contains(&hotkey("PrintScreen")));
    assert_eq!(instance.config().language, Language::En);
}
