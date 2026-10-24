//! 结果面板：显示 OCR 的识别结果，文字可以编辑和复制。
//!
//! 识别语言、识别进度与结果由 `bps_core::ResultPanel` 管理；这里只负责画界面，并在后台线程
//! 执行它给出的识别（`Recognition::run`），完成后交回。识别在本机进行，不发出网络请求；
//! 只有用户点“翻译”时才把文字（不含截图）发给用户配置的翻译服务（ADR 0004）。
//!
//! 界面自上而下：识别语言与识别状态 → 可编辑的识别结果 → 操作按钮（复制……）。

use std::ffi::OsStr;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bps_core::{
    Language, Recognition, RecognitionOutcome, RecognitionStatus, ResultPanel, Texts, Translation,
    TranslationConfig, TranslationOutcome, TranslationStatus,
};
use bps_platform::{HttpTransport, Image, TextRecognizer};
use eframe::egui::{self, RichText};

use crate::{UiEnvironment, WindowSpec, run_window};

/// Windows 设置里“语言和区域”页面，在那里添加语言、安装 OCR 组件。
const LANGUAGE_SETTINGS: &str = "ms-settings:regionlanguage";

/// 结果面板需要的环境。
pub struct ResultPanelEnvironment {
    pub ui: UiEnvironment,
    /// 配置文件里的界面语言。
    pub language: Language,
    /// 要识别文字的截图。
    pub screenshot: Image,
    /// 文字识别；在后台线程里调用。
    pub recognizer: Arc<dyn TextRecognizer + Send + Sync>,
    /// 配置文件里的翻译服务；没有时翻译按钮禁用。
    pub translation: Option<TranslationConfig>,
    /// 翻译请求的 HTTP 传输；在后台线程里调用。
    pub http: Arc<dyn HttpTransport + Send + Sync>,
}

/// 打开结果面板、开始识别，运行到用户关闭它。
pub fn run_result_panel(environment: ResultPanelEnvironment) -> Result<(), String> {
    let ui = environment.ui.clone();
    let texts = ui.texts(environment.language);
    let spec = WindowSpec {
        title: texts.get("result-panel-window-title"),
        inner_size: [560.0, 400.0],
        min_inner_size: [360.0, 240.0],
    };
    run_window(&ui, spec, move |ctx| {
        ResultPanelApp::new(ctx.clone(), environment, texts)
    })
}

struct ResultPanelApp {
    ui: UiEnvironment,
    texts: &'static Texts,
    ctx: egui::Context,
    recognizer: Arc<dyn TextRecognizer + Send + Sync>,
    panel: ResultPanel,
    outcomes: Sender<RecognitionOutcome>,
    finished: Receiver<RecognitionOutcome>,
    http: Arc<dyn HttpTransport + Send + Sync>,
    translated: Sender<TranslationOutcome>,
    translation_finished: Receiver<TranslationOutcome>,
    /// 识别结果出来后把输入焦点放到文本框里一次，方便直接 Ctrl+A、Ctrl+C。
    focus_text: bool,
    notice: Option<Notice>,
}

/// 操作按钮旁边的提示。
enum Notice {
    Copied,
    Failed(String),
}

/// 复制的是哪一段文字。
enum CopyTarget {
    Recognized,
    Translated(String),
}

impl ResultPanelApp {
    fn new(ctx: egui::Context, environment: ResultPanelEnvironment, texts: &'static Texts) -> Self {
        let (outcomes, finished) = mpsc::channel();
        let (translated, translation_finished) = mpsc::channel();
        let (mut panel, recognition) =
            ResultPanel::open(environment.screenshot, environment.recognizer.as_ref());
        let locale = environment
            .language
            .resolve(&environment.ui.system_languages);
        panel.set_translation(environment.translation, locale);
        let mut app = Self {
            ui: environment.ui,
            texts,
            ctx,
            recognizer: environment.recognizer,
            panel,
            outcomes,
            finished,
            http: environment.http,
            translated,
            translation_finished,
            focus_text: false,
            notice: None,
        };
        app.start(recognition);
        app
    }

    /// 在后台线程执行识别，完成后唤醒界面。
    fn start(&mut self, recognition: Option<Recognition>) {
        let Some(recognition) = recognition else {
            return;
        };
        self.notice = None;
        let recognizer = self.recognizer.clone();
        let outcomes = self.outcomes.clone();
        let ctx = self.ctx.clone();
        let spawned = thread::Builder::new()
            .name("text-recognition".to_owned())
            .spawn(move || {
                let _ = outcomes.send(recognition.run(recognizer.as_ref()));
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            self.notice = Some(Notice::Failed(e.to_string()));
        }
    }

    fn receive_outcomes(&mut self) {
        while let Ok(outcome) = self.finished.try_recv() {
            self.panel.finish(outcome);
            if self.panel.status() == &RecognitionStatus::Recognized {
                self.focus_text = true;
            }
        }
    }

    /// 在后台线程发送翻译请求（只含文字），完成后唤醒界面。
    fn start_translation(&mut self) {
        let Some(translation) = self.panel.start_translation() else {
            return;
        };
        self.notice = None;
        let http = self.http.clone();
        let outcomes = self.translated.clone();
        let ctx = self.ctx.clone();
        let spawned = thread::Builder::new()
            .name("translation".to_owned())
            .spawn(move || run_translation(translation, http, outcomes, ctx));
        if let Err(e) = spawned {
            self.notice = Some(Notice::Failed(e.to_string()));
        }
    }

    fn receive_translations(&mut self) {
        while let Ok(outcome) = self.translation_finished.try_recv() {
            self.panel.finish_translation(outcome);
        }
    }

    fn copy(&mut self, target: CopyTarget) {
        let text = match &target {
            CopyTarget::Recognized => self.panel.text(),
            CopyTarget::Translated(text) => text.as_str(),
        };
        self.notice = Some(match (self.ui.clipboard.write_text)(text) {
            Ok(()) => Notice::Copied,
            Err(e) => Notice::Failed(
                self.texts
                    .format("result-panel-copy-failed", &[("error", &e)]),
            ),
        });
    }

    fn open_language_settings(&mut self) {
        if let Err(e) = (self.ui.open)(OsStr::new(LANGUAGE_SETTINGS)) {
            self.notice = Some(Notice::Failed(
                self.texts
                    .format("result-panel-open-settings-failed", &[("error", &e)]),
            ));
        }
    }
}

impl eframe::App for ResultPanelApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_outcomes();
        self.receive_translations();
        let texts = self.texts;

        // 没有可用的识别语言（没装 OCR 语言包、或查询失败）时没有可选的，不显示这一栏。
        if !self.panel.languages().is_empty() {
            egui::Panel::top("recognition")
                .show_separator_line(true)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    self.recognition_bar(ui, texts);
                    ui.add_space(4.0);
                });
        }
        egui::Panel::bottom("actions")
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.add_space(6.0);
                self.actions_bar(ui, texts);
                ui.add_space(4.0);
            });
        if self.panel.translation_status() != &TranslationStatus::Idle {
            egui::Panel::bottom("translation")
                .show_separator_line(true)
                .resizable(true)
                .default_size(130.0)
                .show(ui, |ui| self.translation_area(ui, texts));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.panel.status() == &RecognitionStatus::NoLanguagePack {
                self.no_language_pack(ui, texts);
                return;
            }
            // 失败原因可能很长，放在文本框上方、可以折行。
            if let RecognitionStatus::Failed(error) = self.panel.status() {
                let message = texts.format("result-panel-failed", &[("error", error)]);
                let message = RichText::new(message).color(ui.visuals().error_fg_color);
                ui.add(egui::Label::new(message).wrap());
                ui.add_space(6.0);
            }
            self.text_area(ui, texts);
        });
    }
}

// 各部分界面。
impl ResultPanelApp {
    /// 识别语言（可切换）与识别进度。
    fn recognition_bar(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(texts.get("result-panel-language"));
            let current = self
                .panel
                .language()
                .and_then(|tag| self.panel.languages().iter().find(|l| l.tag == tag))
                .map(|l| l.name.clone())
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("recognition-language")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for language in self.panel.languages() {
                        let selected = self.panel.language() == Some(language.tag.as_str());
                        if ui.selectable_label(selected, &language.name).clicked() {
                            chosen = Some(language.tag.clone());
                        }
                    }
                });
            match self.panel.status() {
                RecognitionStatus::Recognizing => {
                    ui.spinner();
                    ui.label(texts.get("result-panel-recognizing"));
                }
                RecognitionStatus::Recognized if self.panel.text().is_empty() => {
                    ui.label(RichText::new(texts.get("result-panel-no-text")).weak());
                }
                RecognitionStatus::Recognized => {
                    ui.label(RichText::new(texts.get("result-panel-recognized")).weak());
                }
                // 失败原因显示在文本框上方。
                RecognitionStatus::Failed(_) | RecognitionStatus::NoLanguagePack => {}
            }
        });
        if let Some(tag) = chosen {
            let recognition = self.panel.select_language(&tag);
            self.start(recognition);
        }
    }

    /// 可编辑的识别结果。识别进行中不能编辑（结果出来会覆盖它）。
    fn text_area(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let editable = self.panel.status() != &RecognitionStatus::Recognizing;
        let hint = match self.panel.status() {
            RecognitionStatus::Recognized => texts.get("result-panel-no-text"),
            _ => String::new(),
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let response = ui.add_sized(
                    ui.available_size(),
                    egui::TextEdit::multiline(self.panel.text_mut())
                        .interactive(editable)
                        .hint_text(hint)
                        .font(egui::TextStyle::Body),
                );
                if std::mem::take(&mut self.focus_text) {
                    response.request_focus();
                }
                // 改过的文字还没复制：“已复制”的提示不再成立。
                if response.changed() {
                    self.notice = None;
                }
            });
    }

    /// 译文（只读，可选中复制）或翻译失败的原因。
    fn translation_area(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        ui.add_space(4.0);
        match self.panel.translation_status().clone() {
            TranslationStatus::Idle => {}
            TranslationStatus::Translating => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(texts.get("result-panel-translating"));
                });
            }
            TranslationStatus::Failed(error) => {
                let message = texts.format("result-panel-translation-failed", &[("error", &error)]);
                let message = RichText::new(message).color(ui.visuals().error_fg_color);
                ui.add(egui::Label::new(message).wrap());
            }
            TranslationStatus::Translated(text) => {
                ui.label(RichText::new(texts.get("result-panel-translation")).weak());
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut text.as_str())
                                .font(egui::TextStyle::Body),
                        );
                    });
            }
        }
        ui.add_space(4.0);
    }

    /// 系统没有 OCR 语言包：说明原因，引导用户去系统设置安装，装好后重新识别。
    fn no_language_pack(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        ui.add_space(8.0);
        ui.heading(texts.get("result-panel-no-language-pack-title"));
        ui.add_space(6.0);
        ui.add(egui::Label::new(texts.get("result-panel-no-language-pack-body")).wrap());
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new(texts.get("result-panel-open-language-settings")).strong())
                .clicked()
            {
                self.open_language_settings();
            }
            if ui
                .button(texts.get("result-panel-recognize-again"))
                .clicked()
            {
                let recognition = self.panel.recognize_again(self.recognizer.as_ref());
                self.start(recognition);
            }
        });
    }

    /// 操作按钮。结果面板上的其他操作（如翻译）也加在这一排。
    fn actions_bar(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let mut copy = false;
        let mut copy_translation = None;
        let mut translate = false;
        let mut retry = false;
        let mut close = false;
        ui.horizontal(|ui| {
            let has_text = !self.panel.text().is_empty();
            copy = ui
                .add_enabled(
                    has_text,
                    egui::Button::new(RichText::new(texts.get("result-panel-copy")).strong()),
                )
                .clicked();
            let translating = self.panel.translation_status() == &TranslationStatus::Translating;
            let can_translate = self.panel.can_translate();
            translate = ui
                .add_enabled(
                    has_text && can_translate && !translating,
                    egui::Button::new(texts.get("result-panel-translate")),
                )
                .on_disabled_hover_text(if can_translate {
                    String::new()
                } else {
                    texts.get("result-panel-translate-unconfigured")
                })
                .clicked();
            if let TranslationStatus::Translated(text) = self.panel.translation_status()
                && ui
                    .button(texts.get("result-panel-copy-translation"))
                    .clicked()
            {
                copy_translation = Some(text.clone());
            }
            if matches!(self.panel.status(), RecognitionStatus::Failed(_)) {
                retry = ui
                    .button(texts.get("result-panel-recognize-again"))
                    .clicked();
            }
            match &self.notice {
                None => {}
                Some(Notice::Copied) => {
                    ui.label(
                        RichText::new(texts.get("result-panel-copied"))
                            .color(ui.visuals().hyperlink_color),
                    );
                }
                Some(Notice::Failed(reason)) => {
                    ui.label(RichText::new(reason).color(ui.visuals().error_fg_color));
                }
            }
            if !self.panel.can_translate() {
                ui.label(
                    RichText::new(texts.get("result-panel-translate-unconfigured-hint")).weak(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button(texts.get("result-panel-close")).clicked();
            });
        });
        if copy {
            self.copy(CopyTarget::Recognized);
        }
        if let Some(text) = copy_translation {
            self.copy(CopyTarget::Translated(text));
        }
        if translate {
            self.start_translation();
        }
        if retry {
            let recognition = self.panel.recognize_again(self.recognizer.as_ref());
            self.start(recognition);
        }
        if close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn run_translation(
    translation: Translation,
    http: Arc<dyn HttpTransport + Send + Sync>,
    outcomes: Sender<TranslationOutcome>,
    ctx: egui::Context,
) {
    let _ = outcomes.send(translation.run(http.as_ref()));
    ctx.request_repaint();
}
