//! 结果面板：在本机识别截图里的文字（OCR），显示可编辑、可复制的识别结果。
//!
//! 结果面板是独立窗口（子进程），[`ResultPanel`] 是它的界面无关部分：识别语言的选择、
//! 识别的进度与结果。识别可能耗时几百毫秒，所以面板不直接调用文字识别，而是给出要做的
//! [`Recognition`]：界面在后台线程执行 [`Recognition::run`]，再把结果交回
//! [`ResultPanel::finish`]。

use std::sync::Arc;

use bps_platform::{
    Clipboard, ClipboardItem, HttpTransport, Image, PlatformError, RecognitionLanguage,
    RecognizedLine, TextRecognizer,
};

use crate::translation::{self, TranslationConfig, TranslationError};
use crate::{Locale, Texts};

/// 识别进行到哪一步了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognitionStatus {
    /// 正在识别。
    Recognizing,
    /// 识别完成，[`ResultPanel::text`] 是识别结果（可能没有识别出任何文字）。
    Recognized,
    /// 系统没有安装任何 OCR 语言包，无法识别；界面应引导用户去系统设置里安装。
    NoLanguagePack,
    /// 识别失败，附带给人看的原因；[`ResultPanel::text`] 保留之前的内容。
    Failed(String),
}

/// 翻译进行到哪一步了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationStatus {
    /// 还没翻译（或识别结果已更新，之前的译文作废）。
    Idle,
    /// 正在翻译。
    Translating,
    /// 译文。
    Translated(String),
    /// 翻译失败，附带给人看的原因（已按界面语言本地化，不含 API Key）。
    Failed(String),
}

/// 结果面板的界面无关部分。
pub struct ResultPanel {
    screenshot: Arc<Image>,
    languages: Vec<RecognitionLanguage>,
    language: Option<String>,
    status: RecognitionStatus,
    text: String,
    /// 最近一次开始的识别的编号；更早开始的识别晚到的结果直接丢弃。
    latest: u64,
    translation: Option<TranslationConfig>,
    locale: Locale,
    translation_status: TranslationStatus,
    /// 最近一次开始的翻译的编号。
    latest_translation: u64,
}

/// 一次要做的翻译：把当时的文字交给配置的翻译服务。在后台线程执行 [`run`](Self::run)。
pub struct Translation {
    id: u64,
    config: TranslationConfig,
    locale: Locale,
    text: String,
}

/// 一次翻译的结果，交回 [`ResultPanel::finish_translation`]。
pub struct TranslationOutcome {
    id: u64,
    result: Result<String, TranslationError>,
}

impl Translation {
    /// 用 `transport` 发送翻译请求（只含文字）。会阻塞到收到回复或超时。
    pub fn run(self, transport: &dyn HttpTransport) -> TranslationOutcome {
        TranslationOutcome {
            id: self.id,
            result: translation::translate(&self.config, self.locale, &self.text, transport),
        }
    }
}

/// 一次要做的识别：用某种识别语言识别截图。在后台线程执行 [`run`](Self::run)。
pub struct Recognition {
    id: u64,
    screenshot: Arc<Image>,
    language: String,
}

/// 一次识别的结果，交回 [`ResultPanel::finish`]。
pub struct RecognitionOutcome {
    id: u64,
    result: Result<Vec<RecognizedLine>, PlatformError>,
}

impl Recognition {
    /// 这次识别所用的识别语言（标签）。
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 用 `recognizer` 识别截图。可能耗时几百毫秒。
    pub fn run(self, recognizer: &dyn TextRecognizer) -> RecognitionOutcome {
        RecognitionOutcome {
            id: self.id,
            result: recognizer.recognize(&self.screenshot, &self.language),
        }
    }
}

impl ResultPanel {
    /// 打开结果面板：查询系统已安装 OCR 语言包的识别语言，选定跟随系统的默认识别语言，
    /// 返回面板和第一次要做的识别。
    pub fn open(screenshot: Image, recognizer: &dyn TextRecognizer) -> (Self, Option<Recognition>) {
        let mut panel = Self {
            screenshot: Arc::new(screenshot),
            languages: Vec::new(),
            language: None,
            status: RecognitionStatus::Recognizing,
            text: String::new(),
            latest: 0,
            translation: None,
            locale: Locale::ZhCn,
            translation_status: TranslationStatus::Idle,
            latest_translation: 0,
        };
        let recognition = panel.recognize_again(recognizer);
        (panel, recognition)
    }

    /// 重新查询识别语言（用户可能刚在系统设置里安装了 OCR 语言包）并重新识别。
    /// 仍然选定原来的识别语言；它已不可用时改用跟随系统的默认识别语言。
    pub fn recognize_again(&mut self, recognizer: &dyn TextRecognizer) -> Option<Recognition> {
        self.languages = match recognizer.languages() {
            Ok(languages) => languages,
            Err(e) => {
                self.languages.clear();
                self.language = None;
                self.status = RecognitionStatus::Failed(e.message().to_owned());
                return None;
            }
        };
        let installed = |tag: &String| self.languages.iter().any(|l| &l.tag == tag);
        self.language = self
            .language
            .take()
            .filter(installed)
            .or_else(|| recognizer.default_language().filter(installed))
            // 用户的语言首选项里没有一种装了 OCR 语言包时，用装了的第一种。
            .or_else(|| self.languages.first().map(|l| l.tag.clone()));
        self.start_recognition()
    }

    /// 用户选了另一种识别语言：用它重新识别。选的就是当前识别语言、或者它没有安装 OCR
    /// 语言包时什么都不做。
    pub fn select_language(&mut self, tag: &str) -> Option<Recognition> {
        if self.language.as_deref() == Some(tag) || !self.languages.iter().any(|l| l.tag == tag) {
            return None;
        }
        self.language = Some(tag.to_owned());
        self.start_recognition()
    }

    /// 系统已安装 OCR 语言包的全部识别语言。
    pub fn languages(&self) -> &[RecognitionLanguage] {
        &self.languages
    }

    /// 当前选定的识别语言（标签）。
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn status(&self) -> &RecognitionStatus {
        &self.status
    }

    /// 识别结果；用户可以编辑它。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 供界面的文本框直接编辑识别结果。
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.text
    }

    /// 设置翻译服务（来自配置文件，`None` 表示没有配置）和界面语言（译文目标语言缺省跟随它）。
    pub fn set_translation(&mut self, config: Option<TranslationConfig>, locale: Locale) {
        self.translation = config;
        self.locale = locale;
    }

    /// 是否配置了翻译服务。没有时界面应禁用翻译按钮并提示去配置文件里配置。
    pub fn can_translate(&self) -> bool {
        self.translation.is_some()
    }

    pub fn translation_status(&self) -> &TranslationStatus {
        &self.translation_status
    }

    /// 用户点了“翻译”：给出要做的翻译（发送的是此刻文本框里的文字）。
    /// 没有配置翻译服务、文字为空、或正在翻译时什么都不做。
    pub fn start_translation(&mut self) -> Option<Translation> {
        let config = self.translation.clone()?;
        if self.text.trim().is_empty() || self.translation_status == TranslationStatus::Translating
        {
            return None;
        }
        self.translation_status = TranslationStatus::Translating;
        self.latest_translation += 1;
        Some(Translation {
            id: self.latest_translation,
            config,
            locale: self.locale,
            text: self.text.clone(),
        })
    }

    /// 后台的翻译完成了：显示译文或失败原因。已作废的翻译结果直接丢弃。
    pub fn finish_translation(&mut self, outcome: TranslationOutcome) {
        if outcome.id != self.latest_translation {
            return;
        }
        self.translation_status = match outcome.result {
            Ok(text) => TranslationStatus::Translated(text),
            Err(e) => TranslationStatus::Failed(e.localized(Texts::of(self.locale))),
        };
    }

    /// 后台的识别完成了：显示它的结果。更早开始、已被新的识别取代的识别，结果直接丢弃。
    pub fn finish(&mut self, outcome: RecognitionOutcome) {
        if outcome.id != self.latest {
            return;
        }
        match outcome.result {
            Ok(lines) => {
                self.text = compose_text(&lines);
                self.status = RecognitionStatus::Recognized;
            }
            Err(e) => self.status = RecognitionStatus::Failed(e.message().to_owned()),
        }
    }

    fn start_recognition(&mut self) -> Option<Recognition> {
        let Some(language) = self.language.clone() else {
            self.status = RecognitionStatus::NoLanguagePack;
            return None;
        };
        self.status = RecognitionStatus::Recognizing;
        self.latest += 1;
        // 文字将被新的识别结果取代：之前的译文（和还在路上的翻译）作废。
        self.latest_translation += 1;
        self.translation_status = TranslationStatus::Idle;
        Some(Recognition {
            id: self.latest,
            screenshot: self.screenshot.clone(),
            language,
        })
    }
}

/// “识别后自动复制”的结果：结果面板子进程不显示界面，识别后复制文字，
/// 再把它转交回常驻实例（[`crate::Request::OcrCopied`]）去通知用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrCopy {
    /// 识别出的文字已复制到剪贴板；`characters` 是不计空白的字数。
    Copied { characters: usize },
    /// 没有识别出文字，剪贴板没动。
    Empty,
    /// 系统没有安装 OCR 语言包，剪贴板没动。
    NoLanguagePack,
    /// 识别或写剪贴板失败，附带原因；剪贴板没动。
    Failed(String),
}

/// 用跟随系统的默认识别语言识别 `screenshot`，把识别出的文字（与结果面板显示的一样）
/// 复制到剪贴板。没有识别出文字或识别失败时不动剪贴板。会阻塞到识别完成。
pub fn copy_recognized_text(
    screenshot: Image,
    recognizer: &dyn TextRecognizer,
    clipboard: &mut dyn Clipboard,
) -> OcrCopy {
    let (mut panel, recognition) = ResultPanel::open(screenshot, recognizer);
    if let Some(recognition) = recognition {
        panel.finish(recognition.run(recognizer));
    }
    match panel.status() {
        RecognitionStatus::Recognized if panel.text().trim().is_empty() => OcrCopy::Empty,
        RecognitionStatus::Recognized => {
            let characters = panel.text().chars().filter(|c| !c.is_whitespace()).count();
            match clipboard.write(vec![ClipboardItem::Text(panel.text().to_owned())]) {
                Ok(()) => OcrCopy::Copied { characters },
                Err(e) => OcrCopy::Failed(e.message().to_owned()),
            }
        }
        RecognitionStatus::NoLanguagePack => OcrCopy::NoLanguagePack,
        RecognitionStatus::Failed(reason) => OcrCopy::Failed(reason.clone()),
        RecognitionStatus::Recognizing => OcrCopy::Failed(String::new()),
    }
}

/// 识别并翻译后只复制译文；任何失败都不改写剪贴板、不打开面板。
/// 仅在用户明确触发翻译时调用，网络请求只发送识别文字。
pub fn copy_translated_text(
    screenshot: Image,
    recognizer: &dyn TextRecognizer,
    clipboard: &mut dyn Clipboard,
    config: Option<&TranslationConfig>,
    locale: Locale,
    transport: &dyn HttpTransport,
) -> OcrCopy {
    let texts = Texts::of(locale);
    let Some(config) = config else {
        return OcrCopy::Failed(texts.get("result-panel-translation-unconfigured"));
    };
    let (mut panel, recognition) = ResultPanel::open(screenshot, recognizer);
    if let Some(recognition) = recognition {
        panel.finish(recognition.run(recognizer));
    }
    match panel.status() {
        RecognitionStatus::NoLanguagePack => return OcrCopy::NoLanguagePack,
        RecognitionStatus::Failed(reason) => return OcrCopy::Failed(reason.clone()),
        RecognitionStatus::Recognized if !panel.text().trim().is_empty() => {}
        _ => return OcrCopy::Empty,
    }
    let translated = match translation::translate(config, locale, panel.text(), transport) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => return OcrCopy::Failed(texts.get("translation-error-reply")),
        Err(error) => return OcrCopy::Failed(error.localized(texts)),
    };
    let characters = translated.chars().filter(|c| !c.is_whitespace()).count();
    match clipboard.write(vec![ClipboardItem::Text(translated)]) {
        Ok(()) => OcrCopy::Copied { characters },
        Err(error) => OcrCopy::Failed(error.message().to_owned()),
    }
}

/// 把识别出的各行拼成文字：行之间换行；词之间加空格，但与中文、日文相邻处不加
/// （这些文字的词之间本来就没有空格，OCR 却把它们识别成一个个独立的词）。
fn compose_text(lines: &[RecognizedLine]) -> String {
    lines
        .iter()
        .map(|words| compose_line(words))
        .collect::<Vec<_>>()
        .join("\n")
}

fn compose_line(words: &[String]) -> String {
    let mut line = String::new();
    for word in words.iter().filter(|w| !w.is_empty()) {
        if let (Some(before), Some(after)) = (line.chars().last(), word.chars().next())
            && !is_unspaced(before)
            && !is_unspaced(after)
        {
            line.push(' ');
        }
        line.push_str(word);
    }
    half_width_between_ascii(&line)
}

/// 夹在两个英文字母或数字之间的全角标点（中文 OCR 会把“2.0”识别成“2．0”）换成半角。
fn half_width_between_ascii(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let ascii_alphanumeric = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(char::is_ascii_alphanumeric)
    };
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| match c {
            // 全角 ASCII（！到～）与对应的半角字符相差 0xFEE0。
            '\u{FF01}'..='\u{FF5E}'
                if ascii_alphanumeric(i.checked_sub(1)) && ascii_alphanumeric(Some(i + 1)) =>
            {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            }
            _ => c,
        })
        .collect()
}

/// 词之间不用空格分隔的文字：汉字、假名，以及中日文的标点和全角字符；间隔号两边也不加空格。
/// 韩文的词之间有空格，不在此列。
fn is_unspaced(c: char) -> bool {
    matches!(
        c,
        '\u{00B7}'                  // 间隔号（中文 OCR 也会把“2.0”里的点认成它）
        | '\u{2E80}'..='\u{2FDF}'   // 部首
        | '\u{3000}'..='\u{303F}'   // 中日韩符号和标点
        | '\u{3040}'..='\u{30FF}'   // 平假名、片假名
        | '\u{31C0}'..='\u{31FF}'   // 笔画、片假名语音扩展
        | '\u{3400}'..='\u{4DBF}'   // 汉字扩展 A
        | '\u{4E00}'..='\u{9FFF}'   // 中日韩统一表意文字
        | '\u{F900}'..='\u{FAFF}'   // 兼容表意文字
        | '\u{FE30}'..='\u{FE4F}'   // 中日韩兼容形式（竖排标点）
        | '\u{FF00}'..='\u{FFEF}'   // 全角、半角形式（全角标点）
        | '\u{20000}'..='\u{3FFFF}' // 汉字扩展 B 及以后
    )
}
