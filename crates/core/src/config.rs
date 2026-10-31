//! 配置文件 `config.toml`：全局设置 + 热键绑定列表。
//!
//! 格式见 [`DEFAULT_CONFIG`] 开头的注释。其他功能需要新配置项时，在下面的 `Raw*` 结构里
//! 加可选的键（缺省时必须有合理的默认行为），解析后合并进 [`Config`] 或
//! [`CaptureRequest`] 的对应字段（如 [`OutputOptions`]、[`RecordingOptions`]）。

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

use bps_platform::{Hotkey, ParseHotkeyError};
use serde::Deserialize;
use toml::Spanned;

use crate::command::codec_from_config_name;
use crate::i18n::UnknownLanguage;
use crate::translation::{TargetLanguage, TranslationConfig, TranslationProvider};
use crate::{CaptureKind, DirectFunction};
use crate::{
    CaptureMode, CaptureRequest, FilenameTemplate, FilenameTemplateError, HotkeyBinding, Language,
    Locale, OutputFormat, OutputOptions, PostCaptureAction, RecordingFormat, RecordingOptions,
    SaveDir, Texts, VideoQuality,
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
#   language = "auto"                              界面语言：auto（跟随系统）、zh-CN、en
#   check_for_updates = false                      检查更新：只查询 GitHub Releases 上的最新版本并提示，不自动下载
#   autostart = false                              开机自启：true / false
#   ocr_auto_copy = false                          OCR / 翻译后自动复制文字 / 译文，不弹结果面板：true / false
#   ocr_shortcut = "Ctrl+O"                         等待确认界面的 OCR 快捷键
#   translate_shortcut = "Ctrl+T"                   等待确认界面的翻译快捷键
#   录屏的默认参数（每条绑定可以单独覆盖，见下文）：
#   frame_rate = 30                                录屏帧率，缺省时 MP4 为 30、GIF 为 15
#   show_cursor = true                             录屏时显示鼠标指针：true / false
#   video_codec = "h264"                           MP4 的视频编码：h264、h265、av1（后两种需要显卡或系统支持，
#                                                  不支持时自动改用 h264 并提示）
#   video_quality = "standard"                     MP4 的画质：low、standard、high
#   video_bitrate = 8000                           MP4 的自定义码率（千比特每秒），写了就不按 video_quality 计算
#   record_system_audio = true                     MP4 录屏录入系统声音（默认输出设备正在播放的声音）：true / false
#   record_microphone = false                      MP4 录屏录入麦克风（和系统声音混在同一条音轨）：true / false
#   microphone = "..."                             录哪个麦克风（设备标识，在设置界面里选），缺省为系统默认设备
#   run_as_admin = false                           以管理员身份运行：true / false。开启后即使前台是
#                                                  管理员权限的窗口，热键也能生效；开机自启改用计划任务，
#                                                  登录时不弹 UAC 提示
#
# 每个 [[bindings]] 是一条热键绑定：
#   hotkey   热键：Ctrl、Alt、Shift、Win 与一个主键用 + 连接，
#            主键可以是 PrintScreen、F1–F24、A–Z、0–9（字母和数字需搭配 Ctrl、Alt 或 Win）
#   mode     捕获模式：display（显示器）、virtual_desktop（虚拟桌面）、region（区域：拖拽框选，
#            或单击直接选中窗口）
#   after    截图完成后做什么（截图类绑定，缺省为 copy_to_clipboard）：
#              copy_to_clipboard   直接复制到剪贴板并退出截图界面
#              await_confirmation  停留在截图界面，选区上有工具条：标注、录屏、长截屏、OCR、翻译、
#                                  另存为、确认（Enter，复制到剪贴板并退出）、取消（Esc，不复制）
#   function 或者改写 function 作直达功能（与 after 二选一）：record（录屏）、long_screenshot（长截屏）、
#            ocr（OCR），热键直接进入该功能
#   auto_save  true / false，默认 false：截图最终交付时（复制并退出、确认）同时存进保存目录，
#            取消时不保存；与 after 无关。对 long_screenshot 同样有效
# 每条绑定还可以单独覆盖：save_dir、filename_template、output_format（录屏用 mp4 或 gif），
#   以及上面录屏的默认参数：frame_rate、show_cursor、video_codec、video_quality、video_bitrate、
#   record_system_audio、record_microphone、microphone。GIF 不使用编码、画质和声音参数
#
# 翻译（可选，OCR 结果面板里的“翻译”按钮）：在 [translation] 里填写你自己的翻译服务；不填 api_key
# 时按钮禁用。点“翻译”时只会把识别出的文字发给你配置的服务，截图本身绝不上传。把 [translation]
# 写在文件末尾（它下面的键都属于 [translation]）。
#   [translation]
#   provider = "openai"                      服务：openai（OpenAI 兼容接口）或 deepl
#   api_key = "sk-..."                       你的 API Key（不会写入日志）
#   base_url = "https://api.openai.com/v1"   可选：OpenAI 兼容接口的地址（可指向自建服务）；deepl 缺省按密钥选免费/专业版
#   model = "gpt-4o-mini"                    可选：OpenAI 兼容接口使用的模型
#   target_language = "auto"                 可选：目标语言 auto（跟随界面语言）、zh-CN、en、ja、ko、fr、de、es、ru

[[bindings]]
hotkey = "PrintScreen"
mode = "region"
after = "await_confirmation"

[[bindings]]
hotkey = "Shift+PrintScreen"
mode = "display"
after = "copy_to_clipboard"
auto_save = true

[[bindings]]
hotkey = "Ctrl+PrintScreen"
mode = "region"
after = "copy_to_clipboard"

[[bindings]]
hotkey = "Alt+PrintScreen"
mode = "region"
function = "record"
output_format = "mp4"

[[bindings]]
hotkey = "Ctrl+Shift+PrintScreen"
mode = "region"
function = "long_screenshot"

[[bindings]]
hotkey = "Win+Shift+O"
mode = "region"
function = "ocr"
"#;

/// 等待确认界面的实际生效按键；禁用的兼容默认键为 None。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UiShortcuts {
    pub ocr: Option<Hotkey>,
    pub translate: Option<Hotkey>,
}

/// 配置文件的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 全局的自动存储目录、命名模板和截图输出格式；配置文件没写的项是内置默认值。
    /// 热键绑定没有覆盖的项使用这里的值；没有对应热键绑定的 CLI 捕获也使用这里的值。
    pub output: OutputOptions,
    /// 按配置文件中的顺序排列的热键绑定，覆盖项已与全局设置合并。
    pub bindings: Vec<HotkeyBinding>,
    /// 界面语言，默认跟随系统。
    pub language: Language,
    /// 是否检查更新，默认关闭（ADR 0004）：开启后只查询 GitHub Releases 上的最新版本并提示。
    pub check_for_updates: bool,
    /// 开机自启（默认关闭）。
    pub autostart: bool,
    /// 以管理员身份运行（默认关闭）：常驻实例以管理员权限运行，管理员权限的窗口在前台时
    /// 热键也能生效；开机自启改用不弹 UAC 提示的方式。
    pub run_as_admin: bool,
    /// 翻译服务（OpenAI 兼容接口或 DeepL）；没有配置（或没填 `api_key`）时为 `None`，
    /// 结果面板的翻译按钮禁用。
    pub translation: Option<TranslationConfig>,
    /// OCR / 翻译后自动复制文字 / 译文，不弹结果面板（默认关闭）。已合并进各热键绑定的
    /// [`CaptureRequest::ocr_auto_copy`]。
    pub ocr_auto_copy: bool,
    /// 等待确认时的界面快捷键；旧配置与全局热键冲突时为 None。
    pub ocr_shortcut: Option<Hotkey>,
    pub translate_shortcut: Option<Hotkey>,
    /// 全局的录屏参数（帧率、鼠标指针、编码、画质、声音；`format` 总是 `None`）。
    /// 已合并进各热键绑定的 [`CaptureRequest::recording`]：绑定没有覆盖的项用这里的值。
    pub recording: RecordingOptions,
}

impl Config {
    /// 只有 `bindings`、其余设置都取内置默认值的配置。
    pub fn with_bindings(bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            output: OutputOptions::default(),
            bindings,
            language: Language::default(),
            check_for_updates: false,
            autostart: false,
            run_as_admin: false,
            translation: None,
            ocr_auto_copy: false,
            ocr_shortcut: Some("Ctrl+O".parse().unwrap()),
            translate_shortcut: Some("Ctrl+T".parse().unwrap()),
            recording: RecordingOptions::default(),
        }
    }

    /// 解析配置文件的全文。
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        // 记事本可能写入 UTF-8 BOM。
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawConfig = toml::from_str(text).map_err(|e| ConfigError::from_toml(text, &e))?;
        let at = |span: Range<usize>, problem| ConfigError::at(text, span, problem);
        let language = match raw.language {
            Some(name) => name
                .get_ref()
                .parse()
                .map_err(|UnknownLanguage(n)| at(name.span(), ConfigProblem::UnknownLanguage(n)))?,
            None => Language::default(),
        };
        let output_format = match raw.output_format {
            Some(name) => match parse_format(&name, text)? {
                Format::Image(format) => format,
                Format::Recording(_) => {
                    return Err(at(name.span(), ConfigProblem::RecordingFormatInGlobal));
                }
            },
            None => OutputFormat::default(),
        };
        let output = OutputOptions {
            save_dir: raw.save_dir.map_or(SaveDir::Default, SaveDir::Custom),
            filename_template: parse_template(raw.filename_template, text)?.unwrap_or_default(),
            output_format,
        };
        let ocr_auto_copy = raw.ocr_auto_copy.unwrap_or(false);
        let recording = RawRecording {
            frame_rate: raw.frame_rate,
            show_cursor: raw.show_cursor,
            video_codec: raw.video_codec,
            video_quality: raw.video_quality,
            video_bitrate: raw.video_bitrate,
            record_system_audio: raw.record_system_audio,
            record_microphone: raw.record_microphone,
            microphone: raw.microphone,
        }
        .resolve(text)?;
        let mut bindings: Vec<HotkeyBinding> = raw
            .bindings
            .into_iter()
            .map(|binding| binding.resolve(&output, text))
            .collect::<Result<_, _>>()?;
        for binding in &mut bindings {
            binding.capture.ocr_auto_copy = ocr_auto_copy;
            let own = std::mem::take(&mut binding.capture.recording);
            binding.capture.recording = own.or(&recording);
        }
        let shortcut = |value: Option<Spanned<String>>,
                        default: &str|
         -> Result<Option<Hotkey>, ConfigError> {
            let explicit = value.is_some();
            let key = value.as_ref().map_or(default, |v| v.get_ref().as_str());
            let span = value.as_ref().map_or(0..0, |v| v.span());
            let hotkey = parse_hotkey(key).map_err(|e| at(span.clone(), e))?;
            if shortcut_reserved(hotkey) {
                return Err(at(span, ConfigProblem::ShortcutConflict(key.to_owned())));
            }
            if bindings.iter().any(|b| b.hotkey == hotkey) {
                return if explicit {
                    Err(at(span, ConfigProblem::ShortcutConflict(key.to_owned())))
                } else {
                    Ok(None)
                };
            }
            Ok(Some(hotkey))
        };
        let ocr_shortcut = shortcut(raw.ocr_shortcut, "Ctrl+O")?;
        let translate_shortcut = shortcut(raw.translate_shortcut, "Ctrl+T")?;
        if let Some(hotkey) = ocr_shortcut
            && translate_shortcut == Some(hotkey)
        {
            return Err(at(
                0..0,
                ConfigProblem::ShortcutConflict(hotkey.to_string()),
            ));
        }
        let translation = raw
            .translation
            .map(|translation| translation.resolve(text))
            .transpose()?
            .flatten();
        Ok(Config {
            output,
            bindings,
            language,
            check_for_updates: raw.check_for_updates.unwrap_or(false),
            autostart: raw.autostart.unwrap_or(false),
            run_as_admin: raw.run_as_admin.unwrap_or(false),
            translation,
            ocr_auto_copy,
            ocr_shortcut,
            translate_shortcut,
            recording,
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
    problem: ConfigProblem,
}

/// 配置文件里具体哪里不对。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigProblem {
    /// TOML 语法错误、取值类型不符、未知的键等，附带 TOML 解析器的说明（英文，不随界面语言变化）。
    Toml(String),
    InvalidHotkey(ParseHotkeyError),
    ShortcutConflict(String),
    /// 字母、数字热键没有搭配 Ctrl、Alt 或 Win；附带热键原文。
    TypingKeyWithoutModifier(String),
    /// 旧版的 `actions = [...]` 写法已不再支持。
    LegacyActions,
    UnknownMode(String),
    /// `mode = "window"` 已并入 `region`（ADR 0008）。
    WindowModeMerged,
    UnknownAfter(String),
    UnknownFunction(String),
    /// 同一条绑定同时写了 `after` 和 `function`。
    AfterWithFunction,
    /// `auto_save = true` 用在了录屏、OCR 这类不自动保存的直达功能上。
    AutoSaveNotApplicable(String),
    InvalidFilenameTemplate(FilenameTemplateError),
    UnknownOutputFormat(String),
    ImageFormatForRecording,
    RecordingFormatForScreenshot,
    RecordingFormatInGlobal,
    ZeroFrameRate,
    UnknownVideoCodec(String),
    UnknownVideoQuality(String),
    ZeroBitrate,
    UnknownLanguage(String),
    UnknownTranslationProvider(String),
    UnknownTargetLanguage(String),
}

impl ConfigError {
    /// 出错位置所在的行（从 1 开始）；无法定位时为 `None`。
    pub fn line(&self) -> Option<usize> {
        self.line
    }

    /// 出了什么错（中文，不含行号）。
    pub fn message(&self) -> String {
        self.problem.localized(Texts::of(Locale::ZhCn))
    }

    /// 用 `texts` 的语言说明错误，带行号。
    pub fn localized(&self, texts: &Texts) -> String {
        let message = self.problem.localized(texts);
        match self.line {
            Some(line) => texts.format(
                "config-error-at-line",
                &[("line", &line), ("message", &message)],
            ),
            None => message,
        }
    }

    /// 无法解析、也无法定位到行的错误。
    pub(crate) fn unparsable(message: &str) -> Self {
        Self {
            line: None,
            problem: ConfigProblem::Toml(message.trim_end().to_owned()),
        }
    }

    fn from_toml(text: &str, error: &toml::de::Error) -> Self {
        Self {
            line: error.span().map(|span| line_of(text, span.start)),
            problem: ConfigProblem::Toml(error.message().trim_end().to_owned()),
        }
    }

    fn at(text: &str, span: Range<usize>, problem: ConfigProblem) -> Self {
        Self {
            line: Some(line_of(text, span.start)),
            problem,
        }
    }
}

impl ConfigProblem {
    fn localized(&self, texts: &Texts) -> String {
        match self {
            ConfigProblem::Toml(message) => {
                texts.format("config-toml-error", &[("message", message)])
            }
            ConfigProblem::InvalidHotkey(error) => hotkey_error(error, texts),
            ConfigProblem::TypingKeyWithoutModifier(hotkey) => {
                texts.format("config-hotkey-typing-key", &[("hotkey", hotkey)])
            }
            ConfigProblem::ShortcutConflict(key) => {
                texts.format("config-shortcut-conflict", &[("hotkey", key)])
            }
            ConfigProblem::LegacyActions => texts.get("config-legacy-actions"),
            ConfigProblem::UnknownMode(name) => {
                texts.format("config-unknown-mode", &[("mode", name)])
            }
            ConfigProblem::WindowModeMerged => texts.get("config-window-mode-merged"),
            ConfigProblem::UnknownAfter(name) => {
                texts.format("config-unknown-after", &[("after", name)])
            }
            ConfigProblem::UnknownFunction(name) => {
                texts.format("config-unknown-function", &[("function", name)])
            }
            ConfigProblem::AfterWithFunction => texts.get("config-after-with-function"),
            ConfigProblem::AutoSaveNotApplicable(function) => {
                texts.format("config-auto-save-not-applicable", &[("function", function)])
            }
            ConfigProblem::InvalidFilenameTemplate(error) => error.localized(texts),
            ConfigProblem::UnknownOutputFormat(format) => {
                texts.format("config-unknown-format", &[("format", format)])
            }
            ConfigProblem::ImageFormatForRecording => {
                texts.get("config-image-format-for-recording")
            }
            ConfigProblem::RecordingFormatForScreenshot => {
                texts.get("config-recording-format-for-screenshot")
            }
            ConfigProblem::RecordingFormatInGlobal => {
                texts.get("config-recording-format-in-global")
            }
            ConfigProblem::ZeroFrameRate => texts.get("config-zero-frame-rate"),
            ConfigProblem::UnknownVideoCodec(codec) => {
                texts.format("config-unknown-video-codec", &[("codec", codec)])
            }
            ConfigProblem::UnknownVideoQuality(quality) => {
                texts.format("config-unknown-video-quality", &[("quality", quality)])
            }
            ConfigProblem::ZeroBitrate => texts.get("config-zero-bitrate"),
            ConfigProblem::UnknownLanguage(language) => {
                texts.format("config-unknown-language", &[("language", language)])
            }
            ConfigProblem::UnknownTranslationProvider(provider) => texts.format(
                "config-unknown-translation-provider",
                &[("provider", provider)],
            ),
            ConfigProblem::UnknownTargetLanguage(language) => {
                texts.format("config-unknown-target-language", &[("language", language)])
            }
        }
    }
}

/// 用 `texts` 的语言说明热键文本为什么无法解析。
pub(crate) fn hotkey_error(error: &ParseHotkeyError, texts: &Texts) -> String {
    match error {
        ParseHotkeyError::NotAModifier { hotkey, part } => texts.format(
            "config-hotkey-not-a-modifier",
            &[("hotkey", hotkey), ("part", part)],
        ),
        ParseHotkeyError::MissingKey { hotkey } => {
            texts.format("config-hotkey-missing-key", &[("hotkey", hotkey)])
        }
        ParseHotkeyError::UnsupportedKey { hotkey, key } => texts.format(
            "config-hotkey-unsupported-key",
            &[("hotkey", hotkey), ("key", key)],
        ),
    }
}

/// 字节偏移 `offset` 所在的行（从 1 开始）。
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized(Texts::of(Locale::ZhCn)))
    }
}

impl std::error::Error for ConfigError {}

// 需要校验的取值先按原文读进来（带位置），解析成功后再逐项校验，
// 这样出错时既能指出行号，又能按界面语言说明原因。

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    language: Option<Spanned<String>>,
    check_for_updates: Option<bool>,
    autostart: Option<bool>,
    run_as_admin: Option<bool>,
    ocr_auto_copy: Option<bool>,
    ocr_shortcut: Option<Spanned<String>>,
    translate_shortcut: Option<Spanned<String>>,
    frame_rate: Option<Spanned<u32>>,
    show_cursor: Option<bool>,
    video_codec: Option<Spanned<String>>,
    video_quality: Option<Spanned<String>>,
    video_bitrate: Option<Spanned<u32>>,
    record_system_audio: Option<bool>,
    record_microphone: Option<bool>,
    microphone: Option<String>,
    save_dir: Option<PathBuf>,
    filename_template: Option<Spanned<String>>,
    output_format: Option<Spanned<String>>,
    translation: Option<RawTranslation>,
    #[serde(default)]
    bindings: Vec<RawBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTranslation {
    provider: Spanned<String>,
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    target_language: Option<Spanned<String>>,
}

impl RawTranslation {
    /// 校验翻译服务设置；没有填 `api_key` 视为没有配置翻译服务。
    fn resolve(self, text: &str) -> Result<Option<TranslationConfig>, ConfigError> {
        let at = |span: Range<usize>, problem| ConfigError::at(text, span, problem);
        let provider =
            TranslationProvider::from_name(self.provider.get_ref()).ok_or_else(|| {
                at(
                    self.provider.span(),
                    ConfigProblem::UnknownTranslationProvider(self.provider.get_ref().clone()),
                )
            })?;
        let target_language = match &self.target_language {
            Some(name) => TargetLanguage::from_name(name.get_ref()).ok_or_else(|| {
                at(
                    name.span(),
                    ConfigProblem::UnknownTargetLanguage(name.get_ref().clone()),
                )
            })?,
            None => TargetLanguage::Auto,
        };
        let non_blank =
            |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let Some(api_key) = non_blank(self.api_key) else {
            return Ok(None);
        };
        Ok(Some(TranslationConfig {
            provider,
            api_key,
            base_url: non_blank(self.base_url),
            model: non_blank(self.model),
            target_language,
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    hotkey: Spanned<String>,
    mode: Spanned<String>,
    after: Option<Spanned<String>>,
    function: Option<Spanned<String>>,
    auto_save: Option<Spanned<bool>>,
    /// 旧版写法，只用来给出明确的错误。
    actions: Option<Spanned<toml::Value>>,
    save_dir: Option<PathBuf>,
    filename_template: Option<Spanned<String>>,
    output_format: Option<Spanned<String>>,
    frame_rate: Option<Spanned<u32>>,
    show_cursor: Option<bool>,
    video_codec: Option<Spanned<String>>,
    video_quality: Option<Spanned<String>>,
    video_bitrate: Option<Spanned<u32>>,
    record_system_audio: Option<bool>,
    record_microphone: Option<bool>,
    microphone: Option<String>,
}

/// 录屏参数的原文：全局设置和热键绑定里都可以写。
struct RawRecording {
    frame_rate: Option<Spanned<u32>>,
    show_cursor: Option<bool>,
    video_codec: Option<Spanned<String>>,
    video_quality: Option<Spanned<String>>,
    video_bitrate: Option<Spanned<u32>>,
    record_system_audio: Option<bool>,
    record_microphone: Option<bool>,
    microphone: Option<String>,
}

impl RawRecording {
    /// 校验录屏参数（输出格式另行处理）。
    fn resolve(self, text: &str) -> Result<RecordingOptions, ConfigError> {
        let at = |span: Range<usize>, problem| ConfigError::at(text, span, problem);
        let frame_rate = match self.frame_rate {
            Some(rate) if *rate.get_ref() == 0 => {
                return Err(at(rate.span(), ConfigProblem::ZeroFrameRate));
            }
            rate => rate.map(Spanned::into_inner),
        };
        let codec = self
            .video_codec
            .map(|name| {
                codec_from_config_name(name.get_ref()).ok_or_else(|| {
                    at(
                        name.span(),
                        ConfigProblem::UnknownVideoCodec(name.get_ref().clone()),
                    )
                })
            })
            .transpose()?;
        let quality = match (self.video_bitrate, self.video_quality) {
            (Some(bitrate), _) if *bitrate.get_ref() == 0 => {
                return Err(at(bitrate.span(), ConfigProblem::ZeroBitrate));
            }
            (Some(bitrate), _) => Some(VideoQuality::Custom(bitrate.into_inner())),
            (None, Some(name)) => Some(VideoQuality::from_config_name(name.get_ref()).ok_or_else(
                || {
                    at(
                        name.span(),
                        ConfigProblem::UnknownVideoQuality(name.get_ref().clone()),
                    )
                },
            )?),
            (None, None) => None,
        };
        let microphone_device = self
            .microphone
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());
        Ok(RecordingOptions {
            format: None,
            frame_rate,
            show_cursor: self.show_cursor,
            system_audio: self.record_system_audio,
            codec,
            quality,
            microphone: self.record_microphone,
            microphone_device,
        })
    }
}

impl RawBinding {
    /// 校验这条绑定，并把覆盖项与全局设置合并成完整的捕获请求。
    fn resolve(self, global: &OutputOptions, text: &str) -> Result<HotkeyBinding, ConfigError> {
        let at = |span: Range<usize>, problem| ConfigError::at(text, span, problem);
        let hotkey = parse_hotkey(self.hotkey.get_ref()).map_err(|p| at(self.hotkey.span(), p))?;
        if let Some(actions) = &self.actions {
            return Err(at(actions.span(), ConfigProblem::LegacyActions));
        }
        let mode = match self.mode.get_ref().as_str() {
            "window" => return Err(at(self.mode.span(), ConfigProblem::WindowModeMerged)),
            name => CaptureMode::from_config_name(name).ok_or_else(|| {
                at(
                    self.mode.span(),
                    ConfigProblem::UnknownMode(name.to_owned()),
                )
            })?,
        };
        let kind = match (&self.after, &self.function) {
            (Some(_), Some(function)) => {
                return Err(at(function.span(), ConfigProblem::AfterWithFunction));
            }
            (Some(after), None) => CaptureKind::Screenshot(
                PostCaptureAction::from_config_name(after.get_ref()).ok_or_else(|| {
                    at(
                        after.span(),
                        ConfigProblem::UnknownAfter(after.get_ref().clone()),
                    )
                })?,
            ),
            (None, Some(function)) => CaptureKind::Function(
                DirectFunction::from_config_name(function.get_ref()).ok_or_else(|| {
                    at(
                        function.span(),
                        ConfigProblem::UnknownFunction(function.get_ref().clone()),
                    )
                })?,
            ),
            (None, None) => CaptureKind::Screenshot(PostCaptureAction::CopyToClipboard),
        };
        let auto_save = match self.auto_save {
            Some(flag) if *flag.get_ref() && !kind.supports_auto_save() => {
                let CaptureKind::Function(function) = kind else {
                    unreachable!("截图类绑定都支持自动保存")
                };
                return Err(at(
                    flag.span(),
                    ConfigProblem::AutoSaveNotApplicable(function.config_name().to_owned()),
                ));
            }
            flag => flag.is_some_and(Spanned::into_inner),
        };
        let records = kind == CaptureKind::Function(DirectFunction::Record);
        let mut output = global.clone();
        let mut recording = RawRecording {
            frame_rate: self.frame_rate,
            show_cursor: self.show_cursor,
            video_codec: self.video_codec,
            video_quality: self.video_quality,
            video_bitrate: self.video_bitrate,
            record_system_audio: self.record_system_audio,
            record_microphone: self.record_microphone,
            microphone: self.microphone,
        }
        .resolve(text)?;
        if let Some(dir) = self.save_dir {
            output.save_dir = SaveDir::Custom(dir);
        }
        if let Some(template) = parse_template(self.filename_template, text)? {
            output.filename_template = template;
        }
        if let Some(name) = self.output_format {
            match (parse_format(&name, text)?, records) {
                (Format::Image(image), false) => output.output_format = image,
                (Format::Recording(video), true) => recording.format = Some(video),
                (Format::Image(_), true) => {
                    return Err(at(name.span(), ConfigProblem::ImageFormatForRecording));
                }
                (Format::Recording(_), false) => {
                    return Err(at(name.span(), ConfigProblem::RecordingFormatForScreenshot));
                }
            }
        }
        Ok(HotkeyBinding {
            hotkey,
            capture: CaptureRequest {
                mode,
                kind,
                auto_save,
                output,
                recording,
                ocr_auto_copy: false,
            },
        })
    }
}

/// 解析热键并检查它不会吞掉正常输入。
/// 与现有遮罩的实际按键分派保持一致：Ctrl+Z/Y 会忽略 Shift 和 Win。
pub(crate) fn shortcut_reserved(hotkey: Hotkey) -> bool {
    matches!(hotkey.key, bps_platform::Key::Letter('Z' | 'Y'))
        && hotkey.modifiers.ctrl
        && !hotkey.modifiers.alt
}

fn parse_hotkey(text: &str) -> Result<Hotkey, ConfigProblem> {
    let hotkey: Hotkey = text.parse().map_err(ConfigProblem::InvalidHotkey)?;
    if !allows_typing(hotkey) {
        return Err(ConfigProblem::TypingKeyWithoutModifier(text.to_owned()));
    }
    Ok(hotkey)
}

/// 字母和数字键单独或只配 Shift 注册成全局热键会吞掉正常输入，必须搭配 Ctrl、Alt 或 Win。
pub(crate) fn allows_typing(hotkey: Hotkey) -> bool {
    let m = hotkey.modifiers;
    !hotkey.key.is_typing_key() || m.ctrl || m.alt || m.win
}

fn parse_template(
    template: Option<Spanned<String>>,
    text: &str,
) -> Result<Option<FilenameTemplate>, ConfigError> {
    template
        .map(|t| {
            FilenameTemplate::parse(t.get_ref()).map_err(|e| {
                ConfigError::at(text, t.span(), ConfigProblem::InvalidFilenameTemplate(e))
            })
        })
        .transpose()
}

/// 配置文件里 `output_format` 的取值：截图格式或录屏格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Image(OutputFormat),
    Recording(RecordingFormat),
}

impl Format {
    /// 配置文件里的写法（不区分大小写，`jpeg` 也可以）。
    pub(crate) fn from_name(name: &str) -> Option<Format> {
        if let Some(format) = OutputFormat::from_extension(name) {
            return Some(Format::Image(format));
        }
        Some(match name.to_ascii_lowercase().as_str() {
            "mp4" => Format::Recording(RecordingFormat::Mp4),
            "gif" => Format::Recording(RecordingFormat::Gif),
            _ => return None,
        })
    }
}

fn parse_format(name: &Spanned<String>, text: &str) -> Result<Format, ConfigError> {
    Format::from_name(name.get_ref()).ok_or_else(|| {
        ConfigError::at(
            text,
            name.span(),
            ConfigProblem::UnknownOutputFormat(name.get_ref().clone()),
        )
    })
}
