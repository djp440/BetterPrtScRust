//! 配置文件的解析规则：通过 `Config::parse` 这个公共 API 测试。

use std::path::PathBuf;

use bps_core::{
    CaptureMode, Config, DEFAULT_FILENAME_TEMPLATE, OutputFormat, OutputOptions, PostCaptureAction,
    RecordingFormat, RecordingOptions, SaveDir,
};
use bps_platform::{Hotkey, Key, Modifiers};

/// 一条录屏绑定（第 1–4 行），`extra` 从第 5 行开始。
fn binding_with(extra: &str) -> String {
    format!(
        "[[bindings]]\nhotkey = \"Ctrl+F9\"\nmode = \"region\"\nactions = [\"record\"]\n{extra}"
    )
}

/// 一条截图绑定（第 1–4 行），`extra` 从第 5 行开始。
fn screenshot_binding_with(extra: &str) -> String {
    format!(
        "[[bindings]]\nhotkey = \"Ctrl+F8\"\nmode = \"display\"\nactions = [\"copy_to_clipboard\", \"auto_save\"]\n{extra}"
    )
}

#[test]
fn recording_binding_overrides_are_parsed_into_capture_request() {
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
    assert_eq!(capture.output.save_dir, SaveDir::Custom(r"D:\录屏".into()));
    assert_eq!(
        capture.output.filename_template.as_str(),
        "{mode} {yyyy-MM-dd}"
    );
    assert_eq!(
        capture.recording,
        RecordingOptions {
            format: Some(RecordingFormat::Gif),
            frame_rate: Some(12),
            show_cursor: Some(false),
        }
    );
}

#[test]
fn binding_without_overrides_uses_built_in_defaults() {
    let config = Config::parse(&binding_with("")).unwrap();

    let capture = &config.bindings[0].capture;
    assert_eq!(capture.output.save_dir, SaveDir::Default);
    assert_eq!(
        capture.output.filename_template.as_str(),
        DEFAULT_FILENAME_TEMPLATE
    );
    assert_eq!(capture.output.output_format, OutputFormat::Png);
    assert_eq!(capture.recording, RecordingOptions::default());
}

#[test]
fn global_settings_apply_to_bindings_that_do_not_override_them() {
    let text = format!(
        "save_dir = 'E:\\Shots'\nfilename_template = \"shot\"\noutput_format = \"jpeg\"\n\n{}\n{}",
        screenshot_binding_with(""),
        screenshot_binding_with("output_format = \"webp\"\nsave_dir = 'F:\\其他'\n")
            .replace("Ctrl+F8", "Ctrl+F7"),
    );

    let config = Config::parse(&text).unwrap();

    let global = OutputOptions {
        save_dir: SaveDir::Custom(PathBuf::from(r"E:\Shots")),
        filename_template: "shot".parse().unwrap(),
        output_format: OutputFormat::Jpeg,
    };
    assert_eq!(config.output, global);
    assert_eq!(config.bindings[0].capture.output, global);
    let overridden = &config.bindings[1].capture.output;
    assert_eq!(overridden.save_dir, SaveDir::Custom(r"F:\其他".into()));
    assert_eq!(overridden.output_format, OutputFormat::WebP);
    assert_eq!(overridden.filename_template.as_str(), "shot");
}

#[test]
fn screenshot_output_format_names_are_case_insensitive_and_accept_jpeg_alias() {
    let cases = [
        ("png", OutputFormat::Png),
        ("jpg", OutputFormat::Jpeg),
        ("JPEG", OutputFormat::Jpeg),
        ("webp", OutputFormat::WebP),
        ("avif", OutputFormat::Avif),
    ];
    for (name, format) in cases {
        let text = format!("output_format = \"{name}\"\n");
        assert_eq!(
            Config::parse(&text).unwrap().output.output_format,
            format,
            "{name}"
        );
    }
}

#[test]
fn recording_output_format_accepts_mp4_and_gif() {
    for (name, format) in [("mp4", RecordingFormat::Mp4), ("gif", RecordingFormat::Gif)] {
        let text = binding_with(&format!("output_format = \"{name}\"\n"));
        let config = Config::parse(&text).unwrap();
        assert_eq!(
            config.bindings[0].capture.recording.format,
            Some(format),
            "{name}"
        );
    }
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
        (binding_with("frame_rate = 0\n"), "frame_rate 必须大于 0"),
        (
            binding_with("output_format = \"bmp\"\n"),
            "未知的输出格式“bmp”",
        ),
        (binding_with("show_cursor = \"yes\"\n"), "yes"),
        (
            binding_with("filename_template = \"{title}\"\n"),
            "无法识别的占位符 {title}",
        ),
        (
            binding_with("output_format = \"png\"\n"),
            "录屏的 output_format 只能是 mp4 或 gif",
        ),
        (
            screenshot_binding_with("output_format = \"mp4\"\n"),
            "mp4、gif 只用于录屏",
        ),
    ];
    for (text, reason) in cases {
        let (line, message) = parse_error(&text);
        assert_eq!(line, Some(5), "{text}: {message}");
        assert!(message.contains(reason), "{text}: {message}");
    }
}

#[test]
fn invalid_global_settings_are_rejected_with_line_and_reason() {
    let cases = [
        (
            "output_format = \"gif\"\n",
            "录屏格式（mp4、gif）写在 actions 含 record 的绑定里",
        ),
        ("filename_template = \"{yyyy\"\n", "没有对应的 }"),
    ];
    for (global, reason) in cases {
        let (line, message) = parse_error(&format!("# 全局设置\n{global}"));
        assert_eq!(line, Some(2), "{global}: {message}");
        assert!(message.contains(reason), "{global}: {message}");
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
