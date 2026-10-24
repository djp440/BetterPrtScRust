//! 配置文件的解析规则：通过 `Config::parse` 这个公共 API 测试。

use std::path::PathBuf;

use bps_core::{CaptureMode, CaptureOverrides, Config, OutputFormat, PostCaptureAction};
use bps_platform::{Hotkey, Key, Modifiers};

fn binding_with(extra: &str) -> String {
    format!(
        "[[bindings]]\nhotkey = \"Ctrl+F9\"\nmode = \"region\"\nactions = [\"record\"]\n{extra}"
    )
}

#[test]
fn binding_overrides_are_parsed_into_capture_request() {
    let text = binding_with(
        r#"save_dir = 'D:\录屏'
filename_template = "{mode} {yyyy-MM-dd}"
output_format = "gif"
frame_rate = 12
show_cursor = false
"#,
    );

    let config = Config::parse(&text).unwrap();

    let capture = &config.bindings[0].capture;
    assert_eq!(capture.mode, CaptureMode::Region);
    assert_eq!(capture.actions, [PostCaptureAction::Record]);
    assert_eq!(
        capture.overrides,
        CaptureOverrides {
            save_dir: Some(PathBuf::from(r"D:\录屏")),
            filename_template: Some("{mode} {yyyy-MM-dd}".to_owned()),
            output_format: Some(OutputFormat::Gif),
            frame_rate: Some(12),
            show_cursor: Some(false),
        }
    );
}

#[test]
fn binding_without_overrides_uses_none_for_every_override() {
    let config = Config::parse(&binding_with("")).unwrap();

    assert_eq!(
        config.bindings[0].capture.overrides,
        CaptureOverrides::default()
    );
}

#[test]
fn global_storage_settings_are_parsed() {
    let text = format!(
        "save_dir = 'E:\\Shots'\nfilename_template = \"shot\"\noutput_format = \"jpeg\"\n\n{}",
        binding_with("")
    );

    let config = Config::parse(&text).unwrap();

    assert_eq!(config.save_dir, Some(PathBuf::from(r"E:\Shots")));
    assert_eq!(config.filename_template.as_deref(), Some("shot"));
    assert_eq!(config.output_format, Some(OutputFormat::Jpg));
    assert_eq!(config.bindings.len(), 1);
}

#[test]
fn hotkey_text_is_case_insensitive_and_accepts_aliases() {
    let cases = [
        (
            "ctrl+shift+prtsc",
            Modifiers::CTRL.with(Modifiers::SHIFT),
            Key::PrintScreen,
        ),
        (
            "Control + Alt + f12",
            Modifiers::CTRL.with(Modifiers::ALT),
            Key::F(12),
        ),
        (
            "win+shift+o",
            Modifiers::WIN.with(Modifiers::SHIFT),
            Key::Letter('O'),
        ),
        ("Alt+7", Modifiers::ALT, Key::Digit(7)),
        ("PrintScreen", Modifiers::NONE, Key::PrintScreen),
    ];
    for (text, modifiers, key) in cases {
        let config = Config::parse(&binding_with("").replace("Ctrl+F9", text)).unwrap();
        assert_eq!(
            config.bindings[0].hotkey,
            Hotkey::new(modifiers, key),
            "{text}"
        );
    }
}

#[test]
fn empty_file_has_no_bindings_and_no_global_settings() {
    let config = Config::parse("").unwrap();

    assert_eq!(config, Config::with_bindings(Vec::new()));
}

#[test]
fn utf8_bom_written_by_notepad_is_accepted() {
    let config = Config::parse(&format!("\u{feff}{}", binding_with(""))).unwrap();

    assert_eq!(config.bindings.len(), 1);
}

/// 把 `text` 解析失败时报告的行号和消息返回，解析成功时 panic。
fn parse_error(text: &str) -> (Option<usize>, String) {
    let error = Config::parse(text).expect_err("应解析失败");
    (error.line(), error.message().to_owned())
}

#[test]
fn invalid_bindings_are_rejected_with_line_and_reason() {
    let cases = [
        // (配置片段替换, 出错的行, 消息里应包含的内容)
        (("\"Ctrl+F9\"", "\"Ctrl+Foo\""), 2, "Foo"),
        (("\"Ctrl+F9\"", "\"Ctrl+F25\""), 2, "F25"),
        (("\"Ctrl+F9\"", "\"Ctrl+Shift\""), 2, "Shift"),
        (
            ("\"Ctrl+F9\"", "\"Shift+A\""),
            2,
            "字母和数字键需要搭配 Ctrl、Alt 或 Win",
        ),
        (
            ("\"Ctrl+F9\"", "\"7\""),
            2,
            "字母和数字键需要搭配 Ctrl、Alt 或 Win",
        ),
        (("\"region\"", "\"regoin\""), 3, "regoin"),
        (("[\"record\"]", "[\"recrod\"]"), 4, "recrod"),
        (("[\"record\"]", "[]"), 4, "至少要有一个截后动作"),
        (("actions", "acitons"), 4, "acitons"),
    ];
    for ((from, to), line, reason) in cases {
        let (error_line, message) = parse_error(&binding_with("").replacen(from, to, 1));
        assert_eq!(error_line, Some(line), "{to}: {message}");
        assert!(message.contains(reason), "{to}: {message}");
    }
}

#[test]
fn invalid_overrides_are_rejected_with_line_and_reason() {
    let cases = [
        ("frame_rate = 0\n", "frame_rate 必须大于 0"),
        ("output_format = \"bmp\"\n", "bmp"),
        ("show_cursor = \"yes\"\n", "yes"),
    ];
    for (extra, reason) in cases {
        let (line, message) = parse_error(&binding_with(extra));
        assert_eq!(line, Some(5), "{extra}: {message}");
        assert!(message.contains(reason), "{extra}: {message}");
    }
}

#[test]
fn toml_syntax_error_reports_line() {
    let (line, _) = parse_error(&binding_with("frame_rate = = 3\n"));

    assert_eq!(line, Some(5));
}

#[test]
fn binding_missing_mode_is_rejected() {
    let text = "[[bindings]]\nhotkey = \"Ctrl+F9\"\nactions = [\"ocr\"]\n";

    let (_, message) = parse_error(text);

    assert!(message.contains("mode"), "{message}");
}
