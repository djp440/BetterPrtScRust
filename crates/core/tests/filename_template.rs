//! 命名模板：通过 `core` 的公共 API 测试占位符、非法字符替换和语法错误。

use bps_core::{
    CaptureMode, DEFAULT_FILENAME_TEMPLATE, FilenameFields, FilenameTemplate, FilenameTemplateError,
};
use bps_platform::LocalDateTime;

fn fields(window_title: Option<&str>) -> FilenameFields<'_> {
    FilenameFields {
        captured_at: LocalDateTime::new(2026, 3, 7, 8, 9, 5).with_millisecond(42),
        mode: CaptureMode::Display,
        window_title,
    }
}

fn render(template: &str, window_title: Option<&str>) -> String {
    FilenameTemplate::parse(template)
        .unwrap()
        .render(&fields(window_title))
}

#[test]
fn default_filename_template_is_date_and_time_to_the_second() {
    assert_eq!(DEFAULT_FILENAME_TEMPLATE, "{yyyy-MM-dd HH-mm-ss}");
    assert_eq!(
        FilenameTemplate::default().render(&fields(None)),
        "2026-03-07 08-09-05"
    );
}

#[test]
fn filename_template_supports_mode_and_window_title_placeholders() {
    assert_eq!(
        render("{mode}_{window_title}_{yyyy}", Some("记事本")),
        "display_记事本_2026"
    );
}

#[test]
fn date_time_placeholder_supports_short_tokens_and_milliseconds() {
    assert_eq!(render("{yy.M.d H-m-s.fff}", None), "26.3.7 8-9-5.042");
    assert_eq!(render("截图 {yyyy年MM月dd日}", None), "截图 2026年03月07日");
}

#[test]
fn illegal_filename_characters_are_replaced_with_underscore() {
    assert_eq!(
        render("{window_title}", Some(r#"a<b>c:d"e/f\g|h?i*j"#)),
        "a_b_c_d_e_f_g_h_i_j"
    );
    assert_eq!(render("{HH:mm}", None), "08_09");
    assert_eq!(render("{window_title}", Some("tab\there")), "tab_here");
}

#[test]
fn filename_is_trimmed_and_avoids_reserved_device_names() {
    assert_eq!(render("{window_title}", Some("  标题. . ")), "标题");
    assert_eq!(render("{window_title}", Some("con")), "con_");
    assert_eq!(render("{window_title}", Some("NUL.tar")), "NUL_.tar");
    assert_eq!(render("{window_title}", Some("COM1")), "COM1_");
    assert_eq!(render("{window_title}", Some("CONSOLE")), "CONSOLE");
}

#[test]
fn empty_filename_falls_back_to_default_template() {
    assert_eq!(render("{window_title}", None), "2026-03-07 08-09-05");
    assert_eq!(render("{window_title}", Some("...")), "2026-03-07 08-09-05");
}

#[test]
fn very_long_window_title_is_truncated() {
    let title = "长".repeat(500);
    let name = render("{window_title}", Some(&title));
    assert_eq!(name, "长".repeat(200));
}

#[test]
fn literal_braces_are_written_doubled() {
    assert_eq!(render("{{{mode}}}", None), "{display}");
}

#[test]
fn malformed_filename_template_is_rejected() {
    assert_eq!(
        FilenameTemplate::parse("{yyyy-MM-dd"),
        Err(FilenameTemplateError::UnclosedPlaceholder)
    );
    assert_eq!(
        FilenameTemplate::parse("a}b"),
        Err(FilenameTemplateError::UnmatchedClosingBrace)
    );
    for unknown in ["title", "Mode", "yyy", "", "--"] {
        assert_eq!(
            FilenameTemplate::parse(&format!("{{{unknown}}}")),
            Err(FilenameTemplateError::UnknownPlaceholder(
                unknown.to_owned()
            )),
            "{{{unknown}}} 应被拒绝"
        );
    }
}

#[test]
fn filename_template_keeps_its_source_text() {
    let template: FilenameTemplate = "{mode} {HH-mm}".parse().unwrap();
    assert_eq!(template.as_str(), "{mode} {HH-mm}");
    assert_eq!(template.to_string(), "{mode} {HH-mm}");
}

#[test]
fn example_name_uses_a_fixed_example_time_not_the_clock() {
    assert_eq!(
        FilenameTemplate::default().render_example(),
        "2026-01-01 12-34-56"
    );
    assert_eq!(
        FilenameTemplate::parse("{mode}_{HH-mm-ss.fff}")
            .unwrap()
            .render_example(),
        "region_12-34-56.000"
    );
}
