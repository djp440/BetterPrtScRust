//! 设置界面的数据模型：把 `config.toml` 读成可编辑的 [`Settings`]，修改后写回文本。
//!
//! 写回时只改动用户在界面上改过的项，其余内容（注释、空行、界面上没有的覆盖项，
//! 如录屏帧率）原样保留。写回的文件由常驻实例的自动重新加载生效。

use std::path::PathBuf;

use bps_platform::Hotkey;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::config::{Format, allows_typing};
use crate::{CaptureKind, DirectFunction};
use crate::{
    CaptureMode, Config, ConfigError, FilenameTemplate, FilenameTemplateError, Language,
    OutputFormat, SaveDir, Texts,
};

const BOM: char = '\u{feff}';

/// 设置界面上可以修改的全部设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub language: Language,
    pub check_for_updates: bool,
    pub autostart: bool,
    pub run_as_admin: bool,
    /// OCR（直达热键或工具条）识别后自动复制文字，不弹结果面板。
    pub ocr_auto_copy: bool,
    /// 自动存储目录；`None` 表示默认目录（“图片”文件夹下的 `BetterPrtSc`）。
    pub save_dir: Option<PathBuf>,
    /// 命名模板的原文。
    pub filename_template: String,
    /// 截图的输出格式（全局）。
    pub output_format: OutputFormat,
    /// 热键绑定，按配置文件中的顺序排列。
    pub bindings: Vec<BindingSettings>,
}

impl Settings {
    /// 这些设置无法写回的原因；没有问题时为空。设置界面据此在保存前提示。
    pub fn problems(&self) -> Vec<SettingsProblem> {
        let mut problems = Vec::new();
        if let Err(e) = FilenameTemplate::parse(&self.filename_template) {
            problems.push(SettingsProblem::InvalidFilenameTemplate(e));
        }
        if self
            .save_dir
            .as_ref()
            .is_some_and(|dir| dir.to_string_lossy().trim().is_empty())
        {
            problems.push(SettingsProblem::EmptySaveDir);
        }
        for (index, binding) in self.bindings.iter().enumerate() {
            let Some(hotkey) = binding.hotkey else {
                problems.push(SettingsProblem::MissingHotkey { binding: index });
                continue;
            };
            if !allows_typing(hotkey) {
                problems.push(SettingsProblem::TypingKeyWithoutModifier {
                    binding: index,
                    hotkey,
                });
            }
            if let Some(other) = self.conflicting_binding(index) {
                problems.push(SettingsProblem::DuplicateHotkey {
                    binding: index,
                    other,
                    hotkey,
                });
            }
        }
        problems
    }

    /// 第 `index` 条绑定的热键与哪条更靠前的绑定相同（与常驻实例注册时的判定一致：
    /// 后面的那条会注册失败）；没有冲突、或热键已清空时为 `None`。
    pub fn conflicting_binding(&self, index: usize) -> Option<usize> {
        let hotkey = self.bindings.get(index)?.hotkey?;
        self.bindings[..index]
            .iter()
            .position(|earlier| earlier.hotkey == Some(hotkey))
    }
}

/// 设置界面上的一条热键绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSettings {
    /// 热键；录制时按 Backspace / Delete 清空后为 `None`，此时无法保存。
    pub hotkey: Option<Hotkey>,
    pub mode: CaptureMode,
    /// 截图类绑定的截后动作，或直达功能。
    pub kind: CaptureKind,
    /// 自动保存（对录屏、OCR 直达功能无意义，写回时不会写出）。
    pub auto_save: bool,
    /// 读入时它是配置文件里的第几条绑定；新增的绑定为 `None`。
    /// 写回时据此保留这条绑定在界面上没有的覆盖项和注释。
    source: Option<usize>,
}

impl BindingSettings {
    /// 新增的一条热键绑定。
    pub fn new(hotkey: Hotkey, mode: CaptureMode, kind: CaptureKind) -> Self {
        Self {
            hotkey: Some(hotkey),
            mode,
            kind,
            auto_save: false,
            source: None,
        }
    }

    /// 读入时它是配置文件里的第几条绑定（从 0 开始）；新增的绑定为 `None`。
    /// 常驻实例报告的注册失败用的也是这个位置。
    pub fn source(&self) -> Option<usize> {
        self.source
    }
}

/// 设置无法写回的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsProblem {
    InvalidFilenameTemplate(FilenameTemplateError),
    /// 选了自定义存储目录却没有填写。
    EmptySaveDir,
    /// 第 `binding` 条（从 0 开始）绑定的字母、数字热键没有搭配 Ctrl、Alt 或 Win。
    TypingKeyWithoutModifier {
        binding: usize,
        hotkey: Hotkey,
    },
    /// 第 `binding` 条（从 0 开始）绑定没有设置热键。
    MissingHotkey {
        binding: usize,
    },
    /// 第 `binding` 条绑定的热键与更靠前的第 `other` 条相同（从 0 开始）。
    DuplicateHotkey {
        binding: usize,
        other: usize,
        hotkey: Hotkey,
    },
    /// 生成的配置文件无法解析（不应发生）。
    Invalid(ConfigError),
}

impl SettingsProblem {
    /// 用 `texts` 的语言说明问题。
    pub fn localized(&self, texts: &Texts) -> String {
        match self {
            SettingsProblem::InvalidFilenameTemplate(error) => error.localized(texts),
            SettingsProblem::EmptySaveDir => texts.get("settings-problem-empty-save-dir"),
            SettingsProblem::TypingKeyWithoutModifier { binding, hotkey } => texts.format(
                "settings-problem-typing-key",
                &[("index", &(binding + 1)), ("hotkey", hotkey)],
            ),
            SettingsProblem::MissingHotkey { binding } => texts.format(
                "settings-problem-missing-hotkey",
                &[("index", &(binding + 1))],
            ),
            SettingsProblem::DuplicateHotkey {
                binding,
                other,
                hotkey,
            } => texts.format(
                "settings-problem-duplicate-hotkey",
                &[
                    ("index", &(binding + 1)),
                    ("hotkey", hotkey),
                    ("other", &(other + 1)),
                ],
            ),
            SettingsProblem::Invalid(error) => texts.format(
                "settings-problem-invalid",
                &[("error", &error.localized(texts))],
            ),
        }
    }

    /// 问题出在哪条热键绑定（从 0 开始）；与绑定无关时为 `None`。
    pub fn binding(&self) -> Option<usize> {
        match self {
            SettingsProblem::TypingKeyWithoutModifier { binding, .. }
            | SettingsProblem::MissingHotkey { binding }
            | SettingsProblem::DuplicateHotkey { binding, .. } => Some(*binding),
            _ => None,
        }
    }
}

/// 读入的配置文件，以及从中得到的设置。
#[derive(Debug, Clone)]
pub struct SettingsDocument {
    original: String,
    bom: bool,
    document: DocumentMut,
    settings: Settings,
}

impl SettingsDocument {
    /// 读入配置文件的全文。配置文件有错误时无法编辑，返回错误（带行号）。
    pub fn parse(text: &str) -> Result<SettingsDocument, ConfigError> {
        let config = Config::parse(text)?;
        let bom = text.starts_with(BOM);
        let body = text.strip_prefix(BOM).unwrap_or(text);
        let document: DocumentMut = body
            .parse()
            .map_err(|e: toml_edit::TomlError| ConfigError::unparsable(e.message()))?;
        Ok(SettingsDocument {
            original: text.to_owned(),
            bom,
            document,
            settings: settings_of(&config),
        })
    }

    /// 读入时的设置。
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// 读入时的全文。
    pub fn text(&self) -> &str {
        &self.original
    }

    /// 生成写回配置文件的全文：只改动 `settings` 与读入时不同的项。
    ///
    /// 设置有问题时一个也不写，返回全部问题。
    pub fn render(&self, settings: &Settings) -> Result<String, Vec<SettingsProblem>> {
        if *settings == self.settings {
            return Ok(self.original.clone());
        }
        let problems = settings.problems();
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut document = self.document.clone();
        let header = take_header(&mut document);
        self.write_globals(document.as_table_mut(), settings);
        self.write_bindings(document.as_table_mut(), &settings.bindings);
        put_header(&mut document, header);

        let mut text = document.to_string();
        if self.bom {
            text.insert(0, BOM);
        }
        Config::parse(&text).map_err(|e| vec![SettingsProblem::Invalid(e)])?;
        Ok(text)
    }

    fn write_globals(&self, root: &mut Table, settings: &Settings) {
        let old = &self.settings;
        if settings.language != old.language {
            set_value(root, "language", settings.language.config_name().into());
        }
        for (key, new, old) in [
            (
                "check_for_updates",
                settings.check_for_updates,
                old.check_for_updates,
            ),
            ("autostart", settings.autostart, old.autostart),
            ("run_as_admin", settings.run_as_admin, old.run_as_admin),
            ("ocr_auto_copy", settings.ocr_auto_copy, old.ocr_auto_copy),
        ] {
            if new != old {
                set_value(root, key, new.into());
            }
        }
        if settings.save_dir != old.save_dir {
            match &settings.save_dir {
                Some(dir) => set_value(root, "save_dir", dir.to_string_lossy().as_ref().into()),
                None => {
                    root.remove("save_dir");
                }
            }
        }
        if settings.filename_template != old.filename_template {
            set_value(
                root,
                "filename_template",
                settings.filename_template.as_str().into(),
            );
        }
        if settings.output_format != old.output_format {
            set_value(
                root,
                "output_format",
                settings.output_format.extension().into(),
            );
        }
    }

    /// 按 `bindings` 重建 `[[bindings]]`：原有的绑定沿用原来的表（保留注释和覆盖项），
    /// 只改动变了的键；新增的绑定追加新表。
    fn write_bindings(&self, root: &mut Table, bindings: &[BindingSettings]) {
        let originals = take_binding_tables(root);
        let mut tables = ArrayOfTables::new();
        for binding in bindings {
            let source = binding
                .source
                .and_then(|i| Some((originals.get(i)?, self.settings.bindings.get(i)?)));
            let (mut table, old) = match source {
                Some((table, old)) => (table.clone(), Some(old)),
                None => (new_binding_table(), None),
            };
            if let Some(hotkey) = binding.hotkey
                && old.map(|o| o.hotkey) != Some(binding.hotkey)
            {
                set_value(&mut table, "hotkey", hotkey.to_string().into());
            }
            if old.map(|o| o.mode) != Some(binding.mode) {
                set_value(&mut table, "mode", binding.mode.config_name().into());
            }
            if old.map(|o| o.kind) != Some(binding.kind) {
                // `after` 与 `function` 互斥：换了种类要删掉另一个键；旧版的 `actions` 也一并清掉。
                table.remove("actions");
                match binding.kind {
                    CaptureKind::Screenshot(after) => {
                        table.remove("function");
                        set_value(&mut table, "after", after.config_name().into());
                    }
                    CaptureKind::Function(function) => {
                        table.remove("after");
                        set_value(&mut table, "function", function.config_name().into());
                    }
                }
            }
            let auto_save = binding.auto_save && binding.kind.supports_auto_save();
            if old.map(|o| o.auto_save && o.kind.supports_auto_save()) != Some(auto_save) {
                if auto_save {
                    set_value(&mut table, "auto_save", true.into());
                } else {
                    table.remove("auto_save");
                }
            } else if !binding.kind.supports_auto_save() {
                table.remove("auto_save");
            }
            drop_mismatched_output_format(&mut table, binding.kind);
            // 按数组里的顺序输出。
            table.set_position(None);
            tables.push(table);
        }
        if !tables.is_empty() {
            root.insert("bindings", Item::ArrayOfTables(tables));
        }
    }
}

/// 把配置文件全文 `text` 里的 `ocr_auto_copy` 改成 `on`（结果面板上的开关），其余内容原样保留。
/// 配置文件有错误、无法写回时返回 `None`。
pub fn set_ocr_auto_copy(text: &str, on: bool) -> Option<String> {
    let document = SettingsDocument::parse(text).ok()?;
    let mut settings = document.settings().clone();
    settings.ocr_auto_copy = on;
    document.render(&settings).ok()
}

/// 从解析好的配置得到设置。
fn settings_of(config: &Config) -> Settings {
    Settings {
        language: config.language,
        check_for_updates: config.check_for_updates,
        autostart: config.autostart,
        run_as_admin: config.run_as_admin,
        ocr_auto_copy: config.ocr_auto_copy,
        save_dir: match &config.output.save_dir {
            SaveDir::Default => None,
            SaveDir::Custom(dir) => Some(dir.clone()),
        },
        filename_template: config.output.filename_template.as_str().to_owned(),
        output_format: config.output.output_format,
        bindings: config
            .bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| BindingSettings {
                hotkey: Some(binding.hotkey),
                mode: binding.capture.mode,
                kind: binding.capture.kind,
                auto_save: binding.capture.auto_save,
                source: Some(index),
            })
            .collect(),
    }
}

/// 设置一个键的值；键已存在时保留它的位置和行尾注释。
fn set_value(table: &mut Table, key: &str, mut value: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            *value.decor_mut() = old.decor().clone();
            *old = value;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

/// 取出原有的热键绑定表（无论写成 `[[bindings]]` 还是内联数组）。
fn take_binding_tables(root: &mut Table) -> Vec<Table> {
    match root.remove("bindings") {
        Some(Item::ArrayOfTables(tables)) => tables.into_iter().collect(),
        Some(Item::Value(Value::Array(array))) => array
            .into_iter()
            .filter_map(|value| match value {
                Value::InlineTable(table) => Some(table.into_table()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn new_binding_table() -> Table {
    let mut table = Table::new();
    // 与上一条绑定之间空一行。
    table.decor_mut().set_prefix("\n");
    table
}

/// 截后动作变了以后，与之不符的 `output_format` 覆盖项（录屏用 mp4/gif，截图用图像格式）
/// 不再有意义，删掉它，改用全局设置或录屏的默认值。
fn drop_mismatched_output_format(table: &mut Table, kind: CaptureKind) {
    let records = kind == CaptureKind::Function(DirectFunction::Record);
    let mismatched = table
        .get("output_format")
        .and_then(Item::as_str)
        .and_then(Format::from_name)
        .is_some_and(|format| matches!(format, Format::Recording(_)) != records);
    if mismatched {
        table.remove("output_format");
    }
}

/// 文件开头的注释（例如默认配置的格式说明）挂在第一个元素的前缀上。
/// 改动前把它取下来，改动后再挂到新的第一个元素上，保证它始终在文件最前面。
fn take_header(document: &mut DocumentMut) -> String {
    let root = document.as_table_mut();
    if let Some((mut key, _)) = root.iter_mut().find(|(_, item)| item.is_value()) {
        let decor = key.leaf_decor_mut();
        let header = prefix_of(decor.prefix());
        decor.set_prefix("");
        return header;
    }
    if let Some(Item::ArrayOfTables(tables)) = root.get_mut("bindings")
        && let Some(first) = tables.iter_mut().next()
    {
        let header = prefix_of(first.decor().prefix());
        // 它不一定还是第一条，先换成普通的空行分隔。
        first.decor_mut().set_prefix("\n");
        return header;
    }
    let header = document.trailing().as_str().unwrap_or_default().to_owned();
    document.set_trailing("");
    header
}

fn put_header(document: &mut DocumentMut, header: String) {
    if header.is_empty() {
        return;
    }
    let root = document.as_table_mut();
    if let Some((mut key, _)) = root.iter_mut().find(|(_, item)| item.is_value()) {
        let decor = key.leaf_decor_mut();
        let own = prefix_of(decor.prefix());
        decor.set_prefix(join_header(&header, &own));
        return;
    }
    if let Some(Item::ArrayOfTables(tables)) = root.get_mut("bindings")
        && let Some(first) = tables.iter_mut().next()
    {
        let own = prefix_of(first.decor().prefix());
        first.decor_mut().set_prefix(join_header(&header, &own));
        return;
    }
    let trailing = document.trailing().as_str().unwrap_or_default().to_owned();
    document.set_trailing(join_header(&header, &trailing));
}

/// 文件开头的注释后面接上第一个元素自己的注释（去掉它开头的空行）。
fn join_header(header: &str, own: &str) -> String {
    format!("{header}{}", own.trim_start_matches(['\r', '\n']))
}

fn prefix_of(prefix: Option<&toml_edit::RawString>) -> String {
    prefix
        .and_then(toml_edit::RawString::as_str)
        .unwrap_or_default()
        .to_owned()
}
