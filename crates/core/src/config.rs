//! 配置文件 `config.toml`：全局设置 + 热键绑定列表。
//!
//! 格式见 [`DEFAULT_CONFIG`] 开头的注释。其他功能需要新配置项时，在下面的 `Raw*` 结构里
//! 加可选的键（缺省时必须有合理的默认行为），解析后合并进 [`Config`] 或
//! [`CaptureRequest`] 的对应字段（如 [`OutputOptions`]、[`RecordingOptions`]）。

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;
use std::str::FromStr;

use bps_platform::Hotkey;
use serde::{Deserialize, Deserializer, de::Error as _};
use toml::Spanned;

use crate::{
    CaptureMode, CaptureRequest, FilenameTemplate, HotkeyBinding, OutputFormat, OutputOptions,
    PostCaptureAction, RecordingFormat, RecordingOptions, SaveDir,
};

/// 首次运行时写入的配置文件，包含 spec #1 规定的 6 条默认热键绑定。
pub const DEFAULT_CONFIG: &str = r#"# BetterPrtSc 配置文件
#
# 保存后自动重新加载；有错误时继续使用上一次有效的配置，并在托盘通知里指出出错的行。
# 删除本文件后重启 BetterPrtSc，会重新生成这份默认配置。
#
# 全局设置（可选，必须写在第一个 [[bindings]] 之前）：
#   save_dir = 'D:\Screenshots'                    自动存储目录（Windows 路径用单引号）
#   filename_template = "{yyyy-MM-dd HH-mm-ss}"    命名模板
#   output_format = "png"                          截图的输出格式：png、jpg、webp、avif
#   autostart = false                              开机自启：true / false
#   run_as_admin = false                           以管理员身份运行：true / false。开启后即使前台是
#                                                  管理员权限的窗口，热键也能生效；开机自启改用计划任务，
#                                                  登录时不弹 UAC 提示
#
# 每个 [[bindings]] 是一条热键绑定：
#   hotkey   热键：Ctrl、Alt、Shift、Win 与一个主键用 + 连接，
#            主键可以是 PrintScreen、F1–F24、A–Z、0–9（字母和数字需搭配 Ctrl、Alt 或 Win）
#   mode     捕获模式：display（显示器）、virtual_desktop（虚拟桌面）、region（区域）、window（窗口）
#   actions  按顺序执行的截后动作：copy_to_clipboard（复制到剪贴板）、auto_save（自动存储）、
#            annotate（标注）、record（录屏）、long_screenshot（长截屏）、ocr（OCR）、translate（翻译）
# 每条绑定还可以单独覆盖：save_dir、filename_template、output_format（录屏用 mp4 或 gif）、
#   frame_rate（录屏帧率）、show_cursor（录屏时是否显示鼠标指针：true / false）

[[bindings]]
hotkey = "PrintScreen"
mode = "region"
actions = ["annotate"]

[[bindings]]
hotkey = "Shift+PrintScreen"
mode = "display"
actions = ["copy_to_clipboard", "auto_save"]

[[bindings]]
hotkey = "Ctrl+PrintScreen"
mode = "window"
actions = ["copy_to_clipboard", "auto_save"]

[[bindings]]
hotkey = "Alt+PrintScreen"
mode = "region"
actions = ["record"]
output_format = "mp4"

[[bindings]]
hotkey = "Ctrl+Shift+PrintScreen"
mode = "region"
actions = ["long_screenshot"]

[[bindings]]
hotkey = "Win+Shift+O"
mode = "region"
actions = ["ocr"]
"#;

/// 配置文件的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 全局的自动存储目录、命名模板和截图输出格式；配置文件没写的项是内置默认值。
    /// 热键绑定没有覆盖的项使用这里的值；没有对应热键绑定的 CLI 捕获也使用这里的值。
    pub output: OutputOptions,
    /// 按配置文件中的顺序排列的热键绑定，覆盖项已与全局设置合并。
    pub bindings: Vec<HotkeyBinding>,
    /// 开机自启（默认关闭）。
    pub autostart: bool,
    /// 以管理员身份运行（默认关闭）：常驻实例以管理员权限运行，管理员权限的窗口在前台时
    /// 热键也能生效；开机自启改用不弹 UAC 提示的方式。
    pub run_as_admin: bool,
}

impl Config {
    /// 只有 `bindings`、全局设置都取内置默认值的配置。
    pub fn with_bindings(bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            output: OutputOptions::default(),
            bindings,
            autostart: false,
            run_as_admin: false,
        }
    }

    /// 解析配置文件的全文。
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        // 记事本可能写入 UTF-8 BOM。
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawConfig = toml::from_str(text).map_err(|e| ConfigError::from_toml(text, &e))?;
        let output = OutputOptions {
            save_dir: raw.save_dir.map_or(SaveDir::Default, SaveDir::Custom),
            filename_template: raw.filename_template.unwrap_or_default(),
            output_format: raw.output_format.unwrap_or_default(),
        };
        let bindings = raw
            .bindings
            .into_iter()
            .map(|binding| binding.resolve(&output, text))
            .collect::<Result<_, _>>()?;
        Ok(Config {
            output,
            bindings,
            autostart: raw.autostart.unwrap_or(false),
            run_as_admin: raw.run_as_admin.unwrap_or(false),
        })
    }
}

impl Default for Config {
    /// [`DEFAULT_CONFIG`] 描述的配置。
    fn default() -> Self {
        Config::parse(DEFAULT_CONFIG).expect("内置的默认配置必须有效")
    }
}

/// 配置文件无法解析：TOML 语法错误、未知的键、无效的取值等。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    line: Option<usize>,
    message: String,
}

impl ConfigError {
    /// 出错位置所在的行（从 1 开始）；无法定位时为 `None`。
    pub fn line(&self) -> Option<usize> {
        self.line
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn from_toml(text: &str, error: &toml::de::Error) -> Self {
        Self {
            line: error.span().map(|span| line_of(text, span.start)),
            message: error.message().trim_end().to_owned(),
        }
    }

    fn at(text: &str, span: Range<usize>, message: &str) -> Self {
        Self {
            line: Some(line_of(text, span.start)),
            message: message.to_owned(),
        }
    }
}

/// 字节偏移 `offset` 所在的行（从 1 开始）。
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "第 {line} 行：{}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    save_dir: Option<PathBuf>,
    #[serde(default, deserialize_with = "filename_template")]
    filename_template: Option<FilenameTemplate>,
    #[serde(default, deserialize_with = "image_format")]
    output_format: Option<OutputFormat>,
    autostart: Option<bool>,
    run_as_admin: Option<bool>,
    #[serde(default)]
    bindings: Vec<RawBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    #[serde(deserialize_with = "hotkey")]
    hotkey: Hotkey,
    mode: CaptureMode,
    #[serde(deserialize_with = "actions")]
    actions: Vec<PostCaptureAction>,
    save_dir: Option<PathBuf>,
    #[serde(default, deserialize_with = "filename_template")]
    filename_template: Option<FilenameTemplate>,
    #[serde(default, deserialize_with = "any_format")]
    output_format: Option<Spanned<Format>>,
    #[serde(default, deserialize_with = "frame_rate")]
    frame_rate: Option<u32>,
    show_cursor: Option<bool>,
}

impl RawBinding {
    /// 把绑定的覆盖项与全局设置合并成完整的捕获请求。
    fn resolve(self, global: &OutputOptions, text: &str) -> Result<HotkeyBinding, ConfigError> {
        let records = self.actions.contains(&PostCaptureAction::Record);
        let mut output = global.clone();
        let mut recording = RecordingOptions {
            format: None,
            frame_rate: self.frame_rate,
            show_cursor: self.show_cursor,
        };
        if let Some(dir) = self.save_dir {
            output.save_dir = SaveDir::Custom(dir);
        }
        if let Some(template) = self.filename_template {
            output.filename_template = template;
        }
        if let Some(format) = self.output_format {
            match (*format.get_ref(), records) {
                (Format::Image(image), false) => output.output_format = image,
                (Format::Recording(video), true) => recording.format = Some(video),
                (Format::Image(_), true) => {
                    return Err(ConfigError::at(
                        text,
                        format.span(),
                        "录屏的 output_format 只能是 mp4 或 gif",
                    ));
                }
                (Format::Recording(_), false) => {
                    return Err(ConfigError::at(
                        text,
                        format.span(),
                        "mp4、gif 只用于录屏（actions 含 record 的绑定）；截图可用 png、jpg、webp、avif",
                    ));
                }
            }
        }
        Ok(HotkeyBinding {
            hotkey: self.hotkey,
            capture: CaptureRequest {
                mode: self.mode,
                actions: self.actions,
                output,
                recording,
            },
        })
    }
}

/// 配置文件里 `output_format` 的取值：截图格式或录屏格式。
#[derive(Debug, Clone, Copy)]
enum Format {
    Image(OutputFormat),
    Recording(RecordingFormat),
}

impl FromStr for Format {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "png" => Format::Image(OutputFormat::Png),
            "jpg" | "jpeg" => Format::Image(OutputFormat::Jpeg),
            "webp" => Format::Image(OutputFormat::WebP),
            "avif" => Format::Image(OutputFormat::Avif),
            "mp4" => Format::Recording(RecordingFormat::Mp4),
            "gif" => Format::Recording(RecordingFormat::Gif),
            _ => {
                return Err(format!(
                    "未知的输出格式“{name}”：截图可用 png、jpg、webp、avif，录屏可用 mp4、gif"
                ));
            }
        })
    }
}

fn hotkey<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hotkey, D::Error> {
    let text = String::deserialize(deserializer)?;
    let hotkey: Hotkey = text.parse().map_err(D::Error::custom)?;
    let m = hotkey.modifiers;
    if hotkey.key.is_typing_key() && !(m.ctrl || m.alt || m.win) {
        return Err(D::Error::custom(format!(
            "热键“{text}”会吞掉正常输入：字母和数字键需要搭配 Ctrl、Alt 或 Win"
        )));
    }
    Ok(hotkey)
}

fn actions<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<PostCaptureAction>, D::Error> {
    let actions = Vec::<PostCaptureAction>::deserialize(deserializer)?;
    if actions.is_empty() {
        return Err(D::Error::custom("actions 至少要有一个截后动作"));
    }
    Ok(actions)
}

fn filename_template<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FilenameTemplate>, D::Error> {
    let text = String::deserialize(deserializer)?;
    FilenameTemplate::parse(&text)
        .map(Some)
        .map_err(D::Error::custom)
}

/// 全局的 `output_format`：只能是截图格式。
fn image_format<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<OutputFormat>, D::Error> {
    let name = String::deserialize(deserializer)?;
    match name.parse().map_err(D::Error::custom)? {
        Format::Image(format) => Ok(Some(format)),
        Format::Recording(_) => Err(D::Error::custom(
            "全局的 output_format 只用于截图，可用 png、jpg、webp、avif；\
             录屏格式（mp4、gif）写在 actions 含 record 的绑定里",
        )),
    }
}

/// 热键绑定的 `output_format`：截图格式或录屏格式，是否与截后动作相符在合并时检查。
fn any_format<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Spanned<Format>>, D::Error> {
    let name = Spanned::<String>::deserialize(deserializer)?;
    let format = name.get_ref().parse().map_err(D::Error::custom)?;
    Ok(Some(Spanned::new(name.span(), format)))
}

fn frame_rate<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    match u32::deserialize(deserializer)? {
        0 => Err(D::Error::custom("frame_rate 必须大于 0")),
        n => Ok(Some(n)),
    }
}
