//! 配置文件 `config.toml`：全局设置 + 热键绑定列表。
//!
//! 格式见 [`DEFAULT_CONFIG`] 开头的注释。其他功能需要新配置项时，在 [`Config`]
//! （全局设置）或 [`CaptureOverrides`]（每条绑定的覆盖项）里加可选字段，
//! 并在下面的 `Raw*` 结构里加对应的键；缺省时必须有合理的默认行为。

use std::fmt;
use std::path::PathBuf;

use bps_platform::Hotkey;
use serde::{Deserialize, Deserializer, de::Error as _};

use crate::{
    CaptureMode, CaptureOverrides, CaptureRequest, HotkeyBinding, OutputFormat, PostCaptureAction,
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
    /// 全局的自动存储目录；`None` 表示内置默认值（`图片\BetterPrtSc`）。
    pub save_dir: Option<PathBuf>,
    /// 全局的命名模板；`None` 表示内置默认值。
    pub filename_template: Option<String>,
    /// 截图的全局输出格式；`None` 表示内置默认值。
    pub output_format: Option<OutputFormat>,
    /// 按配置文件中的顺序排列的热键绑定。
    pub bindings: Vec<HotkeyBinding>,
}

impl Config {
    /// 只有 `bindings`、全局设置都取内置默认值的配置。
    pub fn with_bindings(bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            save_dir: None,
            filename_template: None,
            output_format: None,
            bindings,
        }
    }

    /// 解析配置文件的全文。
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        // 记事本可能写入 UTF-8 BOM。
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawConfig = toml::from_str(text).map_err(|e| ConfigError::from_toml(text, &e))?;
        Ok(raw.into())
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
        let line = error
            .span()
            .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
        Self {
            line,
            message: error.message().trim_end().to_owned(),
        }
    }
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
    filename_template: Option<String>,
    output_format: Option<OutputFormat>,
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
    filename_template: Option<String>,
    output_format: Option<OutputFormat>,
    #[serde(default, deserialize_with = "frame_rate")]
    frame_rate: Option<u32>,
    show_cursor: Option<bool>,
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        Config {
            save_dir: raw.save_dir,
            filename_template: raw.filename_template,
            output_format: raw.output_format,
            bindings: raw.bindings.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RawBinding> for HotkeyBinding {
    fn from(raw: RawBinding) -> Self {
        HotkeyBinding {
            hotkey: raw.hotkey,
            capture: CaptureRequest {
                mode: raw.mode,
                actions: raw.actions,
                overrides: CaptureOverrides {
                    save_dir: raw.save_dir,
                    filename_template: raw.filename_template,
                    output_format: raw.output_format,
                    frame_rate: raw.frame_rate,
                    show_cursor: raw.show_cursor,
                },
            },
        }
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

fn frame_rate<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    match u32::deserialize(deserializer)? {
        0 => Err(D::Error::custom("frame_rate 必须大于 0")),
        n => Ok(Some(n)),
    }
}
