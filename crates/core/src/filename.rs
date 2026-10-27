//! 命名模板：自动存储时根据日期时间、捕获模式、窗口标题等生成文件名。
//!
//! 语法：`{...}` 是占位符，其余文字原样保留；`{{`、`}}` 表示字面的花括号。
//! - `{mode}`：捕获模式，与 CLI 的名称一致：`display`、`virtual-desktop`、`region`、`window`
//! - `{window_title}`：被捕获窗口的标题，没有窗口时为空
//! - 其他占位符是日期时间格式，由下列记号和任意非字母字符组成（区分大小写）：
//!   `yyyy` `yy` 年，`MM` `M` 月，`dd` `d` 日，`HH` `H` 时（24 小时制），
//!   `mm` `m` 分，`ss` `s` 秒，`fff` 毫秒。例如 `{yyyy-MM-dd HH-mm-ss}`。
//!
//! 生成的文件名里，Windows 文件名不允许的字符替换成 `_`。

use std::fmt;
use std::str::FromStr;

use bps_platform::LocalDateTime;

use crate::{CaptureMode, Locale, Texts};
// `{mode}` 的取值与 CLI 里的捕获模式名称一致（display、virtual-desktop、region、window）。
use crate::request::mode_name;

/// 默认命名模板。
pub const DEFAULT_FILENAME_TEMPLATE: &str = "{yyyy-MM-dd HH-mm-ss}";

/// 设置界面预览命名模板用的固定示例时间：2026-01-01 12:34:56。
pub const EXAMPLE_CAPTURE_TIME: LocalDateTime = LocalDateTime::new(2026, 1, 1, 12, 34, 56);

/// 文件名主干（不含扩展名）的最大长度（UTF-16 码元），给目录、`-N` 后缀和扩展名留出余量。
const MAX_STEM_UTF16: usize = 200;

/// 解析好的命名模板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilenameTemplate {
    source: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Text(String),
    DateTime(Vec<DateTimePart>),
    Mode,
    WindowTitle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DateTimePart {
    Text(char),
    Year4,
    Year2,
    Month2,
    Month1,
    Day2,
    Day1,
    Hour2,
    Hour1,
    Minute2,
    Minute1,
    Second2,
    Second1,
    Millisecond3,
}

/// 生成文件名所需的信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilenameFields<'a> {
    /// 捕获发生时的本地时间。
    pub captured_at: LocalDateTime,
    pub mode: CaptureMode,
    /// 被捕获窗口的标题；不是单击选中的窗口时为 `None`。
    pub window_title: Option<&'a str>,
}

/// 命名模板语法错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilenameTemplateError {
    /// `{` 没有对应的 `}`。
    UnclosedPlaceholder,
    /// 单独出现的 `}`（字面的 `}` 要写成 `}}`）。
    UnmatchedClosingBrace,
    /// 无法识别的占位符，例如 `{title}`。
    UnknownPlaceholder(String),
}

impl FilenameTemplateError {
    /// 用 `texts` 的语言说明错误。
    pub fn localized(&self, texts: &Texts) -> String {
        match self {
            FilenameTemplateError::UnclosedPlaceholder => texts.get("template-unclosed"),
            FilenameTemplateError::UnmatchedClosingBrace => texts.get("template-unmatched-closing"),
            FilenameTemplateError::UnknownPlaceholder(name) => {
                texts.format("template-unknown-placeholder", &[("name", name)])
            }
        }
    }
}

impl fmt::Display for FilenameTemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized(Texts::of(Locale::ZhCn)))
    }
}

impl std::error::Error for FilenameTemplateError {}

impl FilenameTemplate {
    pub fn parse(template: &str) -> Result<Self, FilenameTemplateError> {
        let mut segments = Vec::new();
        let mut text = String::new();
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    text.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    text.push('}');
                }
                '}' => return Err(FilenameTemplateError::UnmatchedClosingBrace),
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some('{') | None => {
                                return Err(FilenameTemplateError::UnclosedPlaceholder);
                            }
                            Some(c) => name.push(c),
                        }
                    }
                    if !text.is_empty() {
                        segments.push(Segment::Text(std::mem::take(&mut text)));
                    }
                    segments.push(placeholder(&name)?);
                }
                c => text.push(c),
            }
        }
        if !text.is_empty() {
            segments.push(Segment::Text(text));
        }
        Ok(Self {
            source: template.to_owned(),
            segments,
        })
    }

    /// 模板原文，可原样写回配置文件。
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// 生成文件名主干（不含扩展名）。
    ///
    /// 不允许出现在文件名里的字符替换成 `_`；结果为空时改用默认命名模板。
    pub fn render(&self, fields: &FilenameFields<'_>) -> String {
        self.render_inner(fields)
    }

    /// 用固定的示例时间（[`EXAMPLE_CAPTURE_TIME`]）、区域模式、无窗口标题渲染，
    /// 设置界面据此预览命名效果；结果不随当前时间变化。
    pub fn render_example(&self) -> String {
        self.render_inner(&FilenameFields {
            captured_at: EXAMPLE_CAPTURE_TIME,
            mode: CaptureMode::Region,
            window_title: None,
        })
    }

    fn render_inner(&self, fields: &FilenameFields<'_>) -> String {
        let mut name = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text(text) => name.push_str(text),
                Segment::DateTime(parts) => format_date_time(&mut name, parts, fields.captured_at),
                Segment::Mode => name.push_str(mode_name(fields.mode)),
                Segment::WindowTitle => name.push_str(fields.window_title.unwrap_or("")),
            }
        }
        let name = sanitize(&name);
        if name.is_empty() && self.source != DEFAULT_FILENAME_TEMPLATE {
            return FilenameTemplate::default().render_inner(fields);
        }
        name
    }
}

impl Default for FilenameTemplate {
    fn default() -> Self {
        Self::parse(DEFAULT_FILENAME_TEMPLATE).expect("默认命名模板应合法")
    }
}

impl FromStr for FilenameTemplate {
    type Err = FilenameTemplateError;

    fn from_str(template: &str) -> Result<Self, Self::Err> {
        Self::parse(template)
    }
}

impl fmt::Display for FilenameTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

fn placeholder(name: &str) -> Result<Segment, FilenameTemplateError> {
    match name {
        "mode" => Ok(Segment::Mode),
        "window_title" => Ok(Segment::WindowTitle),
        _ => date_time_format(name)
            .map(Segment::DateTime)
            .ok_or_else(|| FilenameTemplateError::UnknownPlaceholder(name.to_owned())),
    }
}

/// 把日期时间格式拆成记号；含有无法识别的字母、或一个记号都没有时返回 `None`。
fn date_time_format(format: &str) -> Option<Vec<DateTimePart>> {
    let chars: Vec<char> = format.chars().collect();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !c.is_ascii_alphabetic() {
            parts.push(DateTimePart::Text(c));
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|&&x| x == c).count();
        let part = match (c, run) {
            ('y', 4) => DateTimePart::Year4,
            ('y', 2) => DateTimePart::Year2,
            ('M', 2) => DateTimePart::Month2,
            ('M', 1) => DateTimePart::Month1,
            ('d', 2) => DateTimePart::Day2,
            ('d', 1) => DateTimePart::Day1,
            ('H', 2) => DateTimePart::Hour2,
            ('H', 1) => DateTimePart::Hour1,
            ('m', 2) => DateTimePart::Minute2,
            ('m', 1) => DateTimePart::Minute1,
            ('s', 2) => DateTimePart::Second2,
            ('s', 1) => DateTimePart::Second1,
            ('f', 3) => DateTimePart::Millisecond3,
            _ => return None,
        };
        parts.push(part);
        i += run;
    }
    parts
        .iter()
        .any(|p| !matches!(p, DateTimePart::Text(_)))
        .then_some(parts)
}

fn format_date_time(out: &mut String, parts: &[DateTimePart], t: LocalDateTime) {
    use std::fmt::Write;
    for part in parts {
        let _ = match part {
            DateTimePart::Text(c) => write!(out, "{c}"),
            DateTimePart::Year4 => write!(out, "{:04}", t.year),
            DateTimePart::Year2 => write!(out, "{:02}", t.year % 100),
            DateTimePart::Month2 => write!(out, "{:02}", t.month),
            DateTimePart::Month1 => write!(out, "{}", t.month),
            DateTimePart::Day2 => write!(out, "{:02}", t.day),
            DateTimePart::Day1 => write!(out, "{}", t.day),
            DateTimePart::Hour2 => write!(out, "{:02}", t.hour),
            DateTimePart::Hour1 => write!(out, "{}", t.hour),
            DateTimePart::Minute2 => write!(out, "{:02}", t.minute),
            DateTimePart::Minute1 => write!(out, "{}", t.minute),
            DateTimePart::Second2 => write!(out, "{:02}", t.second),
            DateTimePart::Second1 => write!(out, "{}", t.second),
            DateTimePart::Millisecond3 => write!(out, "{:03}", t.millisecond),
        };
    }
}

/// 让文件名主干在 Windows 上合法：非法字符换成 `_`，去掉首尾空白和结尾的点，
/// 避开 `CON`、`NUL` 等设备名，并限制长度。
fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();

    let mut stem = String::new();
    let mut utf16_len = 0;
    for c in replaced.trim().chars() {
        utf16_len += c.len_utf16();
        if utf16_len > MAX_STEM_UTF16 {
            break;
        }
        stem.push(c);
    }
    let mut stem = stem.trim_end_matches(['.', ' ']).trim_start().to_owned();

    // 设备名后面带扩展名也是保留的（`NUL.txt`），所以把 `_` 插在第一个点之前。
    let base_len = stem.find('.').unwrap_or(stem.len());
    if is_reserved_device_name(&stem[..base_len]) {
        stem.insert(base_len, '_');
    }
    stem
}

/// Windows 保留的设备名（不区分大小写）。
fn is_reserved_device_name(base: &str) -> bool {
    let base = base.trim_end().to_ascii_uppercase();
    match base.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            (base.starts_with("COM") || base.starts_with("LPT"))
                && base.len() == 4
                && matches!(base.as_bytes()[3], b'1'..=b'9')
        }
    }
}
