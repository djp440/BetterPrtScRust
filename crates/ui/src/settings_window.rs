//! 设置界面：读取 `config.toml` → 在界面上修改 → 写回，常驻实例自动重新加载后立即生效。
//!
//! 注册失败的热键绑定与检查更新的结果来自常驻实例：后台线程经命名管道反复发 `status`
//! 请求。设置界面自己不发任何网络请求。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use bps_core::{
    BindingFailure, BindingSettings, CaptureKind, CaptureMode, FilenameTemplate, Language,
    OutputFormat, PostCaptureAction, ResidentStatus, Settings, SettingsDocument, SettingsProblem,
    Texts, UpdateStatus, VERSION, VideoQuality,
};
use bps_platform::{AudioDevice, Hotkey, Key, Modifiers, VideoCodec};
use eframe::egui::{self, RichText};

use crate::hotkey_recording::{
    Conclusion, PauseHotkeys, Progress, RecordingSession, StartKeyCapture,
};
use crate::store::{self, LoadProblem, Loaded, SaveError};
use crate::{UiEnvironment, WindowSpec, run_window};

/// 查询常驻实例状态的间隔。
const STATUS_INTERVAL: Duration = Duration::from_secs(1);

/// 弹出“选择文件夹”对话框（标题、起始目录），取消时返回 `None`。
pub type PickFolder = Arc<dyn Fn(&str, Option<&Path>) -> Option<PathBuf> + Send + Sync>;

/// 探测这台电脑的录屏能力（要试着建编码器，约需一秒）。在后台线程里调用。
pub type ProbeRecording = Arc<dyn Fn() -> RecordingCapabilities + Send + Sync>;

/// 这台电脑的录屏能力。
#[derive(Debug, Clone)]
pub struct RecordingCapabilities {
    /// 不能用的视频编码和原因。
    pub unsupported_codecs: Vec<(VideoCodec, String)>,
    /// 系统里没有解码器的视频编码：能录，但系统播放器放不了。
    pub undecodable_codecs: Vec<VideoCodec>,
    /// 可用的麦克风；列不出来时是原因。
    pub microphones: Result<Vec<AudioDevice>, String>,
}

/// 自定义码率的默认值和范围（千比特每秒）。
const DEFAULT_CUSTOM_BITRATE: u32 = 8_000;
const CUSTOM_BITRATE_RANGE: std::ops::RangeInclusive<u32> = 300..=100_000;
/// 帧率下拉框里的常用值。
const FRAME_RATES: [u32; 4] = [15, 24, 30, 60];

/// 设置界面需要的环境。
pub struct SettingsEnvironment {
    pub ui: UiEnvironment,
    pub config_file: PathBuf,
    /// 查询常驻实例的状态；常驻实例没在运行（或查询失败）时返回 `None`。
    /// 在后台线程里反复调用。
    pub query_status: Box<dyn FnMut() -> Option<ResidentStatus> + Send>,
    /// 弹出“选择文件夹”对话框（标题、起始目录），取消时返回 `None`。在后台线程里调用。
    pub pick_folder: PickFolder,
    /// 录制热键时开始捕获按键（低层键盘钩子，能拿到 PrintScreen 和 Win 组合）。
    pub key_capture: StartKeyCapture,
    /// 录制热键期间暂停（续租）/ 恢复常驻实例的全局热键，免得按下的组合键触发截图。
    pub pause_hotkeys: PauseHotkeys,
    /// 普通权限的程序能改写本程序（例如装在用户自己的目录里）。
    /// 这时开启“以管理员身份运行”有提权风险，界面上给出提示。
    pub program_writable_by_standard_users: bool,
    /// 探测录屏能力（可用的视频编码、麦克风）。
    pub probe_recording: ProbeRecording,
}

/// 打开设置界面，运行到用户关闭它。
pub fn run_settings_window(environment: SettingsEnvironment) -> Result<(), String> {
    let loaded = store::load(&environment.config_file);
    let language = match &loaded {
        Loaded::Editable { document, .. } => document.settings().language,
        Loaded::Broken(_) => Language::Auto,
    };
    let ui = environment.ui.clone();
    let spec = WindowSpec {
        title: ui.texts(language).get("settings-window-title"),
        inner_size: [660.0, 760.0],
        min_inner_size: [540.0, 420.0],
    };
    run_window(&ui, spec, move |ctx| {
        SettingsApp::new(ctx.clone(), environment, loaded)
    })
}

struct SettingsApp {
    ui: UiEnvironment,
    config_file: PathBuf,
    pick_folder: PickFolder,
    key_capture: StartKeyCapture,
    pause_hotkeys: PauseHotkeys,
    program_writable_by_standard_users: bool,
    probe_recording: ProbeRecording,

    page: Page,
    group: Group,
    /// 正在进行的热键录制。
    recording: Option<RecordingSession>,
    /// 上一帧录制中的热键框的位置：点到它之外就取消录制。
    record_rect: Option<egui::Rect>,
    /// 无法开始录制的原因。
    record_error: Option<String>,
    resident: Resident,
    statuses: Receiver<Option<ResidentStatus>>,
    picked_folder: Option<Receiver<Option<PathBuf>>>,
    /// 录屏能力；第一次打开录屏分组时才探测。
    capabilities: Option<RecordingCapabilities>,
    probing: Option<Receiver<RecordingCapabilities>>,
    notice: Option<Notice>,
    asking_to_close: bool,
    may_close: bool,
    /// 当前的窗口标题，语言变了才更新。
    title: String,
}

/// 左侧的分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    General,
    Hotkeys,
    Saving,
    Recording,
    About,
}

impl Group {
    const ALL: [Group; 5] = [
        Group::General,
        Group::Hotkeys,
        Group::Saving,
        Group::Recording,
        Group::About,
    ];

    fn label(self, texts: &Texts) -> String {
        texts.get(match self {
            Group::General => "settings-group-general",
            Group::Hotkeys => "settings-group-hotkeys",
            Group::Saving => "settings-group-saving",
            Group::Recording => "settings-group-recording",
            Group::About => "settings-group-about",
        })
    }
}

/// 项目主页。
const PROJECT_URL: &str = "https://github.com/djp440/BetterPrtScRust";

enum Page {
    Editing(Box<Editor>),
    Broken(LoadProblem),
}

struct Editor {
    document: SettingsDocument,
    /// 读入时文件的全文（不存在时为 `None`），保存前据此确认没被别人改过。
    on_disk: Option<String>,
    /// 界面上正在修改的设置。
    settings: Settings,
    /// 自定义存储目录输入框的内容；切到默认目录再切回来时保留。
    custom_dir: String,
    /// 自定义码率输入框的值（千比特每秒）；切到档位再切回来时保留。
    custom_bitrate: u32,
    /// 保存时发现配置文件已被别人改过。
    changed_elsewhere: bool,
}

impl Editor {
    fn new(document: SettingsDocument, on_disk: Option<String>) -> Self {
        let settings = document.settings().clone();
        let custom_dir = settings
            .save_dir
            .as_ref()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        let custom_bitrate = match settings.recording.quality {
            VideoQuality::Custom(kbps) => kbps,
            _ => DEFAULT_CUSTOM_BITRATE,
        };
        Editor {
            document,
            on_disk,
            settings,
            custom_dir,
            custom_bitrate,
            changed_elsewhere: false,
        }
    }

    fn is_modified(&self) -> bool {
        self.settings != *self.document.settings()
    }
}

/// 常驻实例的状态。
enum Resident {
    /// 还没查询到。
    Unknown,
    NotRunning,
    Running(ResidentStatus),
}

/// 底部的提示。
enum Notice {
    Saved,
    SaveFailed(String),
    OpenFailed(String),
}

impl SettingsApp {
    fn new(ctx: egui::Context, environment: SettingsEnvironment, loaded: Loaded) -> Self {
        let SettingsEnvironment {
            ui,
            config_file,
            query_status,
            pick_folder,
            key_capture,
            pause_hotkeys,
            program_writable_by_standard_users,
            probe_recording,
        } = environment;
        let page = match loaded {
            Loaded::Editable { document, on_disk } => {
                Page::Editing(Box::new(Editor::new(*document, on_disk)))
            }
            Loaded::Broken(problem) => Page::Broken(problem),
        };
        SettingsApp {
            ui,
            config_file,
            pick_folder,
            key_capture,
            pause_hotkeys,
            program_writable_by_standard_users,
            probe_recording,
            page,
            group: Group::General,
            recording: None,
            record_rect: None,
            record_error: None,
            resident: Resident::Unknown,
            statuses: poll_status(query_status, ctx),
            picked_folder: None,
            capabilities: None,
            probing: None,
            notice: None,
            asking_to_close: false,
            may_close: false,
            title: String::new(),
        }
    }

    fn texts(&self) -> &'static Texts {
        let language = match &self.page {
            // 界面语言跟着正在修改的设置走，选了就切换。
            Page::Editing(editor) => editor.settings.language,
            Page::Broken(_) => Language::Auto,
        };
        self.ui.texts(language)
    }

    fn is_modified(&self) -> bool {
        matches!(&self.page, Page::Editing(editor) if editor.is_modified())
    }

    /// 录制热键：处理钩子收到的按键，有结论就写进正在编辑的绑定；点到热键框之外则取消。
    fn pump_recording(&mut self, ctx: &egui::Context) {
        let Some(session) = self.recording.as_mut() else {
            return;
        };
        let clicked_away = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_none_or(|pos| self.record_rect.is_none_or(|rect| !rect.contains(pos)))
        });
        if session.has_concluded() {
            if session.drain_finished() {
                self.recording = None;
            }
        } else if clicked_away {
            self.recording = None;
        } else if let Progress::Concluded(conclusion) = session.poll() {
            let index = session.binding;
            if let Page::Editing(editor) = &mut self.page
                && let Some(binding) = editor.settings.bindings.get_mut(index)
            {
                match conclusion {
                    Conclusion::Recorded(hotkey) => binding.hotkey = Some(hotkey),
                    Conclusion::Cleared => binding.hotkey = None,
                    Conclusion::Cancelled => {}
                }
            }
        }
        if self.recording.is_some() {
            // 钩子线程没法唤醒界面，录制期间定时刷新来取按键。
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    fn start_recording(&mut self, binding: usize) {
        self.record_error = None;
        match RecordingSession::start(binding, &self.key_capture, self.pause_hotkeys.clone()) {
            Ok(session) => self.recording = Some(session),
            Err(error) => self.record_error = Some(error),
        }
    }

    fn reload(&mut self) {
        self.recording = None;
        self.page = match store::load(&self.config_file) {
            Loaded::Editable { document, on_disk } => {
                Page::Editing(Box::new(Editor::new(*document, on_disk)))
            }
            Loaded::Broken(problem) => Page::Broken(problem),
        };
        self.notice = None;
    }

    /// 写回配置文件；成功时返回 `true`。
    fn save(&mut self) -> bool {
        let Page::Editing(editor) = &mut self.page else {
            return false;
        };
        let texts = self.ui.texts(editor.settings.language);
        let text = match editor.document.render(&editor.settings) {
            Ok(text) => text,
            Err(problems) => {
                let reasons: Vec<String> = problems.iter().map(|p| p.localized(texts)).collect();
                self.notice = Some(Notice::SaveFailed(reasons.join("\n")));
                return false;
            }
        };
        match store::save(&self.config_file, editor.on_disk.as_deref(), &text) {
            Ok(()) => {
                match SettingsDocument::parse(&text) {
                    Ok(document) => **editor = Editor::new(document, Some(text)),
                    // render 已经确认过写回的文本可以解析，不应发生。
                    Err(e) => self.page = Page::Broken(LoadProblem::Invalid(e)),
                }
                self.notice = Some(Notice::Saved);
                true
            }
            Err(SaveError::ChangedOnDisk) => {
                editor.changed_elsewhere = true;
                self.notice = None;
                false
            }
            Err(SaveError::Io(e)) => {
                self.notice = Some(Notice::SaveFailed(e.to_string()));
                false
            }
        }
    }

    fn open(&mut self, target: &std::ffi::OsStr) {
        if let Err(e) = (self.ui.open)(target) {
            self.notice = Some(Notice::OpenFailed(e));
        }
    }

    fn open_config_folder(&mut self) {
        if let Some(dir) = self.config_file.parent().map(Path::to_path_buf) {
            // 首次运行前目录可能还不存在。
            let _ = std::fs::create_dir_all(&dir);
            self.open(dir.as_os_str());
        }
    }

    fn receive_background_results(&mut self) {
        while let Ok(status) = self.statuses.try_recv() {
            self.resident = match status {
                Some(status) => Resident::Running(status),
                None => Resident::NotRunning,
            };
        }
        if let Some(receiver) = &self.picked_folder
            && let Ok(picked) = receiver.try_recv()
        {
            self.picked_folder = None;
            if let (Some(dir), Page::Editing(editor)) = (picked, &mut self.page) {
                editor.custom_dir = dir.display().to_string();
                editor.settings.save_dir = Some(dir);
            }
        }
        if let Some(receiver) = &self.probing
            && let Ok(capabilities) = receiver.try_recv()
        {
            self.probing = None;
            self.capabilities = Some(capabilities);
        }
    }

    /// 在后台线程里探测录屏能力，探测完刷新界面。
    fn start_probing(&mut self, ctx: &egui::Context) {
        if self.probing.is_some() {
            return;
        }
        let probe = self.probe_recording.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("bps-probe-recording".to_owned())
            .spawn(move || {
                let _ = sender.send(probe());
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.probing = Some(receiver);
        }
    }

    fn start_picking_folder(&mut self, ctx: &egui::Context, title: String) {
        let Page::Editing(editor) = &self.page else {
            return;
        };
        let initial = editor.settings.save_dir.clone();
        let pick = self.pick_folder.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("bps-pick-folder".to_owned())
            .spawn(move || {
                let _ = sender.send(pick(&title, initial.as_deref()));
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.picked_folder = Some(receiver);
        }
    }
}

/// 在后台线程里反复查询常驻实例的状态，有变化时交给界面线程并刷新界面。
fn poll_status(
    mut query: Box<dyn FnMut() -> Option<ResidentStatus> + Send>,
    ctx: egui::Context,
) -> Receiver<Option<ResidentStatus>> {
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("bps-resident-status".to_owned())
        .spawn(move || {
            let mut last = None;
            loop {
                let status = query();
                if last.as_ref() != Some(&status) {
                    if sender.send(status.clone()).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    last = Some(status);
                }
                thread::sleep(STATUS_INTERVAL);
            }
        });
    receiver
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_background_results();
        self.pump_recording(&ctx);
        let texts = self.texts();

        let title = texts.get("settings-window-title");
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.is_modified() && !self.may_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.asking_to_close = true;
        }

        egui::Panel::bottom("actions")
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.add_space(6.0);
                self.bottom_bar(ui, texts);
                ui.add_space(4.0);
            });
        if matches!(self.page, Page::Editing(_)) {
            egui::Panel::left("groups")
                .resizable(false)
                .exact_size(150.0)
                .show(ui, |ui| self.group_list(ui, texts));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| match &self.page {
                    Page::Editing(_) => self.editing_page(ui, texts),
                    Page::Broken(_) => self.broken_page(ui, texts),
                });
        });

        if self.asking_to_close {
            self.close_dialog(&ctx, texts);
        }
    }
}

// 各部分界面。
impl SettingsApp {
    fn editing_page(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        match &self.resident {
            Resident::NotRunning => {
                ui.add(
                    egui::Label::new(
                        RichText::new(texts.get("settings-resident-not-running"))
                            .color(ui.visuals().warn_fg_color),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
            }
            Resident::Unknown | Resident::Running(_) => {}
        }
        match self.group {
            Group::General => self.general_section(ui, texts),
            Group::Hotkeys => self.bindings_section(ui, texts),
            Group::Saving => self.saving_section(ui, texts),
            Group::Recording => self.recording_section(ui, texts),
            Group::About => self.about_section(ui, texts),
        }
        ui.add_space(8.0);
    }

    fn group_list(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        ui.add_space(8.0);
        let mut chosen = self.group;
        for group in Group::ALL {
            let button = egui::Button::selectable(self.group == group, group.label(texts))
                .min_size(egui::vec2(ui.available_width(), 30.0));
            if ui.add(button).clicked() {
                chosen = group;
            }
        }
        if chosen != self.group {
            self.group = chosen;
            // 离开热键页就结束录制。
            self.recording = None;
            if chosen == Group::Recording && self.capabilities.is_none() {
                let ctx = ui.ctx().clone();
                self.start_probing(&ctx);
            }
        }
    }

    fn recording_section(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let probing = self.probing.is_some();
        let mut refresh = false;
        let capabilities = self.capabilities.clone();
        let Page::Editing(editor) = &mut self.page else {
            return;
        };
        let settings = &mut editor.settings.recording;
        ui.heading(texts.get("settings-section-recording"));
        hint(ui, texts.get("settings-recording-hint"));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(texts.get("settings-recording-frame-rate"));
            let label = |rate: Option<u32>| match rate {
                Some(rate) => texts.format("settings-recording-fps", &[("fps", &rate)]),
                None => texts.get("settings-recording-frame-rate-default"),
            };
            egui::ComboBox::from_id_salt("frame-rate")
                .selected_text(label(settings.frame_rate))
                .show_ui(ui, |ui| {
                    let mut choices: Vec<Option<u32>> = vec![None];
                    choices.extend(FRAME_RATES.map(Some));
                    // 配置文件里手写的其他帧率也列出来，免得打开下拉框就丢了。
                    if !choices.contains(&settings.frame_rate) {
                        choices.push(settings.frame_rate);
                    }
                    for rate in choices {
                        ui.selectable_value(&mut settings.frame_rate, rate, label(rate));
                    }
                });
        });
        ui.checkbox(
            &mut settings.show_cursor,
            texts.get("settings-recording-show-cursor"),
        );

        ui.add_space(10.0);
        ui.label(RichText::new(texts.get("settings-recording-mp4")).strong());
        hint(ui, texts.get("settings-recording-mp4-hint"));
        let unsupported = |codec: VideoCodec| -> Option<String> {
            match &capabilities {
                Some(capabilities) => capabilities
                    .unsupported_codecs
                    .iter()
                    .find(|(c, _)| *c == codec)
                    .map(|(_, reason)| reason.clone()),
                // 没探测完之前只能选 H.264。
                None if codec != VideoCodec::H264 => Some(texts.get("settings-recording-probing")),
                None => None,
            }
        };
        ui.horizontal(|ui| {
            ui.label(texts.get("settings-recording-codec"));
            egui::ComboBox::from_id_salt("video-codec")
                .selected_text(settings.codec.label())
                .show_ui(ui, |ui| {
                    for codec in VideoCodec::ALL {
                        let reason = unsupported(codec);
                        let item = ui.add_enabled(
                            reason.is_none() || settings.codec == codec,
                            egui::Button::selectable(settings.codec == codec, codec.label()),
                        );
                        if item.clicked() {
                            settings.codec = codec;
                        }
                        if let Some(reason) = reason {
                            item.on_disabled_hover_text(&reason).on_hover_text(reason);
                        }
                    }
                });
            if probing {
                ui.spinner();
            }
        });
        if capabilities.is_some()
            && let Some(reason) = unsupported(settings.codec)
        {
            warning(
                ui,
                texts.format(
                    "settings-recording-codec-unsupported",
                    &[("codec", &settings.codec.label()), ("reason", &reason)],
                ),
            );
        }
        if let Some(capabilities) = &capabilities
            && unsupported(settings.codec).is_none()
            && capabilities.undecodable_codecs.contains(&settings.codec)
        {
            warning(
                ui,
                texts.format(
                    "settings-recording-codec-no-decoder",
                    &[("codec", &settings.codec.label())],
                ),
            );
        }
        if let Some(capabilities) = &capabilities {
            for (codec, reason) in &capabilities.unsupported_codecs {
                hint(
                    ui,
                    texts.format(
                        "settings-recording-codec-unavailable",
                        &[("codec", &codec.label()), ("reason", reason)],
                    ),
                );
            }
        }

        ui.add_space(4.0);
        ui.label(texts.get("settings-recording-quality"));
        let mut quality = settings.quality;
        ui.horizontal_wrapped(|ui| {
            for (preset, key) in [
                (VideoQuality::Low, "settings-recording-quality-low"),
                (
                    VideoQuality::Standard,
                    "settings-recording-quality-standard",
                ),
                (VideoQuality::High, "settings-recording-quality-high"),
            ] {
                ui.radio_value(&mut quality, preset, texts.get(key));
            }
        });
        ui.horizontal(|ui| {
            let custom = matches!(quality, VideoQuality::Custom(_));
            if ui
                .radio(custom, texts.get("settings-recording-quality-custom"))
                .clicked()
            {
                quality = VideoQuality::Custom(editor.custom_bitrate);
            }
            let field = ui.add_enabled(
                custom,
                egui::DragValue::new(&mut editor.custom_bitrate)
                    .range(CUSTOM_BITRATE_RANGE)
                    .speed(50.0)
                    .suffix(" kbps"),
            );
            if field.changed() && custom {
                quality = VideoQuality::Custom(editor.custom_bitrate);
            }
        });
        settings.quality = quality;
        hint(ui, texts.get("settings-recording-quality-hint"));

        ui.add_space(4.0);
        ui.checkbox(
            &mut settings.system_audio,
            texts.get("settings-recording-system-audio"),
        );
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut settings.microphone,
                texts.get("settings-recording-microphone"),
            );
            let devices: &[AudioDevice] = match capabilities.as_ref().map(|c| &c.microphones) {
                Some(Ok(devices)) => devices,
                _ => &[],
            };
            let device_label = |id: &Option<String>| match id {
                None => texts.get("settings-recording-microphone-default"),
                Some(id) => devices
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| {
                        texts.format("settings-recording-microphone-missing", &[("id", id)])
                    }),
            };
            ui.add_enabled_ui(settings.microphone, |ui| {
                egui::ComboBox::from_id_salt("microphone")
                    .selected_text(device_label(&settings.microphone_device))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        let mut choices: Vec<Option<String>> = vec![None];
                        choices.extend(devices.iter().map(|d| Some(d.id.clone())));
                        if !choices.contains(&settings.microphone_device) {
                            choices.push(settings.microphone_device.clone());
                        }
                        for id in choices {
                            let label = device_label(&id);
                            ui.selectable_value(&mut settings.microphone_device, id, label);
                        }
                    });
            });
            if ui
                .add_enabled(
                    !probing,
                    egui::Button::new(texts.get("settings-recording-refresh")),
                )
                .clicked()
            {
                refresh = true;
            }
        });
        match capabilities.as_ref().map(|c| &c.microphones) {
            Some(Err(reason)) => warning(
                ui,
                texts.format(
                    "settings-recording-microphones-failed",
                    &[("error", reason)],
                ),
            ),
            Some(Ok(devices)) => {
                if let Some(id) = &settings.microphone_device
                    && !devices.iter().any(|d| &d.id == id)
                {
                    warning(ui, texts.get("settings-recording-microphone-fallback"));
                }
            }
            None => {}
        }
        hint(ui, texts.get("settings-recording-audio-hint"));

        if refresh {
            let ctx = ui.ctx().clone();
            self.start_probing(&ctx);
        }
    }

    fn about_section(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        ui.heading(texts.get("settings-group-about"));
        ui.add_space(4.0);
        ui.label("BetterPrtSc");
        ui.label(texts.format("settings-version", &[("version", &VERSION)]));
        ui.label(texts.format(
            "settings-about-license",
            &[("license", &env!("CARGO_PKG_LICENSE"))],
        ));
        ui.add_space(4.0);
        if ui.link(texts.get("settings-about-project")).clicked() {
            self.open(PROJECT_URL.as_ref());
        }
        hint(ui, PROJECT_URL.to_owned());
    }

    fn general_section(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let Page::Editing(editor) = &mut self.page else {
            return;
        };
        let saved = editor.document.settings().clone();
        let settings = &mut editor.settings;
        ui.heading(texts.get("settings-section-general"));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(texts.get("settings-language"));
            egui::ComboBox::from_id_salt("language")
                .selected_text(language_label(settings.language, texts))
                .show_ui(ui, |ui| {
                    for language in Language::ALL {
                        ui.selectable_value(
                            &mut settings.language,
                            language,
                            language_label(language, texts),
                        );
                    }
                });
        });

        ui.checkbox(&mut settings.autostart, texts.get("settings-autostart"));
        ui.checkbox(
            &mut settings.run_as_admin,
            texts.get("settings-run-as-admin"),
        );
        hint(ui, texts.get("settings-run-as-admin-hint"));
        if settings.run_as_admin && self.program_writable_by_standard_users {
            warning(ui, texts.get("settings-run-as-admin-risk"));
        }

        ui.checkbox(
            &mut settings.check_for_updates,
            texts.get("settings-check-for-updates"),
        );
        hint(ui, texts.get("settings-check-for-updates-hint"));
        let mut open_page = None;
        if settings.check_for_updates
            && saved.check_for_updates
            && let Resident::Running(status) = &self.resident
        {
            match &status.update {
                UpdateStatus::Disabled => {}
                UpdateStatus::Checking => hint(ui, texts.get("settings-update-checking")),
                UpdateStatus::UpToDate => hint(
                    ui,
                    texts.format("settings-update-up-to-date", &[("version", &VERSION)]),
                ),
                UpdateStatus::Available { version, page_url } => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(
                                texts.format("settings-update-available", &[("version", version)]),
                            )
                            .strong(),
                        );
                        if ui.button(texts.get("settings-update-open-page")).clicked() {
                            open_page = Some(page_url.clone());
                        }
                    });
                }
                UpdateStatus::Failed(reason) => warning(
                    ui,
                    texts.format("settings-update-failed", &[("error", reason)]),
                ),
            }
        }

        ui.add_space(12.0);
        ui.heading(texts.get("settings-section-ocr"));
        ui.add_space(4.0);
        ui.checkbox(
            &mut settings.ocr_auto_copy,
            texts.get("settings-ocr-auto-copy"),
        );
        hint(ui, texts.get("settings-ocr-auto-copy-hint"));
        if let Some(url) = open_page {
            self.open(url.as_ref());
        }
    }

    fn saving_section(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let picking = self.picked_folder.is_some();
        let mut browse = false;
        let Page::Editing(editor) = &mut self.page else {
            return;
        };
        let settings = &mut editor.settings;
        ui.heading(texts.get("settings-section-saving"));
        ui.add_space(4.0);

        ui.label(texts.get("settings-save-dir"));
        let mut custom = settings.save_dir.is_some();
        ui.radio_value(&mut custom, false, texts.get("settings-save-dir-default"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut custom, true, texts.get("settings-save-dir-custom"));
            let field = ui.add_enabled(
                custom,
                egui::TextEdit::singleline(&mut editor.custom_dir).desired_width(300.0),
            );
            if field.changed() {
                settings.save_dir = Some(PathBuf::from(&editor.custom_dir));
            }
            if ui
                .add_enabled(
                    custom && !picking,
                    egui::Button::new(texts.get("settings-browse")),
                )
                .clicked()
            {
                browse = true;
            }
        });
        if custom != settings.save_dir.is_some() {
            settings.save_dir = custom.then(|| PathBuf::from(&editor.custom_dir));
        }
        if settings.problems().contains(&SettingsProblem::EmptySaveDir) {
            error(ui, SettingsProblem::EmptySaveDir.localized(texts));
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(texts.get("settings-filename-template"));
            ui.add(
                egui::TextEdit::singleline(&mut settings.filename_template)
                    .desired_width(f32::INFINITY),
            );
        });
        match FilenameTemplate::parse(&settings.filename_template) {
            Ok(template) => {
                // 固定的示例时间，预览不随秒数变化。
                let name = template.render_example();
                hint(
                    ui,
                    texts.format(
                        "settings-filename-example",
                        &[(
                            "name",
                            &format!("{name}.{}", settings.output_format.extension()),
                        )],
                    ),
                );
            }
            Err(e) => error(ui, e.localized(texts)),
        }
        hint(ui, texts.get("settings-filename-help"));

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(texts.get("settings-output-format"));
            egui::ComboBox::from_id_salt("output-format")
                .selected_text(format_label(settings.output_format))
                .show_ui(ui, |ui| {
                    for format in OUTPUT_FORMATS {
                        ui.selectable_value(
                            &mut settings.output_format,
                            format,
                            format_label(format),
                        );
                    }
                });
        });
        hint(ui, texts.get("settings-output-format-hint"));

        if browse {
            let title = texts.get("settings-choose-folder");
            let ctx = ui.ctx().clone();
            self.start_picking_folder(&ctx, title);
        }
    }

    fn bindings_section(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let failures: &[BindingFailure] = match &self.resident {
            Resident::Running(status) => &status.registration_failures,
            Resident::Unknown | Resident::NotRunning => &[],
        };
        let recording = self
            .recording
            .as_ref()
            .filter(|r| !r.has_concluded())
            .map(|r| RecordingView {
                binding: r.binding,
                preview: r.preview,
                unsupported: r.unsupported,
            });
        let record_error = self.record_error.clone();
        let Page::Editing(editor) = &mut self.page else {
            return;
        };
        let settings = &mut editor.settings;
        ui.heading(texts.get("settings-section-bindings"));
        hint(ui, texts.get("settings-bindings-hint"));
        ui.add_space(4.0);

        if let Some(message) = record_error {
            error(
                ui,
                texts.format("settings-record-unavailable", &[("error", &message)]),
            );
        }
        let problems = settings.problems();
        let mut removed = None;
        let mut start_recording = None;
        let mut record_rect = None;
        for (index, binding) in settings.bindings.iter_mut().enumerate() {
            // 常驻实例报告的失败按配置文件里的位置和热键对应到界面上的绑定。
            let failure = failures
                .iter()
                .find(|f| Some(f.index) == binding.source() && Some(f.hotkey) == binding.hotkey);
            let reasons: Vec<String> = failure
                .map(|f| {
                    texts.format(
                        "settings-binding-failed",
                        &[("reason", &f.error.localized(texts))],
                    )
                })
                .into_iter()
                .chain(
                    problems
                        .iter()
                        .filter(|p| p.binding() == Some(index))
                        .map(|p| binding_problem(p, texts)),
                )
                .collect();
            let outcome = binding_editor(ui, texts, index, binding, &reasons, recording);
            if outcome.remove {
                removed = Some(index);
            }
            if outcome.start_recording {
                start_recording = Some(index);
            }
            if outcome.record_rect.is_some() {
                record_rect = outcome.record_rect;
            }
            ui.add_space(4.0);
        }
        if let Some(index) = removed {
            settings.bindings.remove(index);
            self.recording = None;
        }
        if ui.button(texts.get("settings-binding-add")).clicked() {
            let hotkey = unused_hotkey(&settings.bindings);
            settings.bindings.push(BindingSettings::new(
                hotkey,
                CaptureMode::Region,
                CaptureKind::Screenshot(PostCaptureAction::CopyToClipboard),
            ));
        }
        self.record_rect = record_rect;
        if let Some(index) = start_recording {
            self.start_recording(index);
        }
    }

    fn broken_page(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let Page::Broken(problem) = &self.page else {
            return;
        };
        ui.heading(texts.get("settings-config-broken"));
        ui.add_space(4.0);
        error(ui, problem.localized(texts));
        ui.add_space(4.0);
        hint(ui, texts.get("settings-config-broken-hint"));
        ui.label(self.config_file.display().to_string());
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui, texts: &Texts) {
        let mut save = false;
        let mut revert = false;
        let mut reload = false;
        let mut open_folder = false;
        match &self.page {
            Page::Editing(editor) => {
                if editor.changed_elsewhere {
                    warning(ui, texts.get("settings-changed-elsewhere"));
                }
                let problems = editor.settings.problems();
                if !problems.is_empty() {
                    error(ui, texts.get("settings-fix-problems"));
                }
                self.notice_line(ui, texts);
                ui.horizontal(|ui| {
                    let modified = editor.is_modified();
                    save = ui
                        .add_enabled(
                            modified && problems.is_empty() && !editor.changed_elsewhere,
                            egui::Button::new(RichText::new(texts.get("settings-save")).strong()),
                        )
                        .clicked();
                    revert = ui
                        .add_enabled(modified, egui::Button::new(texts.get("settings-revert")))
                        .clicked();
                    if editor.changed_elsewhere {
                        reload = ui.button(texts.get("settings-reload")).clicked();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        open_folder = ui
                            .button(texts.get("settings-open-config-folder"))
                            .clicked();
                    });
                });
            }
            Page::Broken(_) => {
                self.notice_line(ui, texts);
                ui.horizontal(|ui| {
                    reload = ui.button(texts.get("settings-reload")).clicked();
                    open_folder = ui
                        .button(texts.get("settings-open-config-folder"))
                        .clicked();
                });
            }
        }
        if save {
            self.save();
        }
        if revert && let Page::Editing(editor) = &mut self.page {
            let document = editor.document.clone();
            let on_disk = editor.on_disk.take();
            **editor = Editor::new(document, on_disk);
            self.notice = None;
        }
        if reload {
            self.reload();
        }
        if open_folder {
            self.open_config_folder();
        }
    }

    fn notice_line(&self, ui: &mut egui::Ui, texts: &Texts) {
        match &self.notice {
            None => {}
            Some(Notice::Saved) => {
                let key = match self.resident {
                    Resident::NotRunning => "settings-saved-not-running",
                    Resident::Unknown | Resident::Running(_) => "settings-saved",
                };
                ui.label(RichText::new(texts.get(key)).color(ui.visuals().hyperlink_color));
            }
            Some(Notice::SaveFailed(reason)) => {
                error(
                    ui,
                    texts.format("settings-save-failed", &[("error", reason)]),
                );
            }
            Some(Notice::OpenFailed(reason)) => error(ui, reason.clone()),
        }
    }

    fn close_dialog(&mut self, ctx: &egui::Context, texts: &Texts) {
        let can_save = matches!(
            &self.page,
            Page::Editing(editor) if editor.settings.problems().is_empty() && !editor.changed_elsewhere
        );
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("unsaved-changes")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(texts.get("settings-unsaved-title"));
            ui.add_space(4.0);
            ui.label(texts.get("settings-unsaved-body"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_save,
                        egui::Button::new(texts.get("settings-save-and-close")),
                    )
                    .clicked()
                {
                    choice = Some(CloseChoice::Save);
                }
                if ui.button(texts.get("settings-discard-and-close")).clicked() {
                    choice = Some(CloseChoice::Discard);
                }
                if ui.button(texts.get("settings-cancel")).clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(CloseChoice::Cancel);
        }
        match choice {
            None => {}
            Some(CloseChoice::Cancel) => self.asking_to_close = false,
            Some(CloseChoice::Save) => {
                self.asking_to_close = false;
                if self.save() {
                    self.close(ctx);
                }
            }
            Some(CloseChoice::Discard) => {
                self.asking_to_close = false;
                self.close(ctx);
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.may_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

/// 正在进行的热键录制中界面要显示的部分。
#[derive(Clone, Copy)]
struct RecordingView {
    binding: usize,
    preview: Modifiers,
    unsupported: bool,
}

/// 一条热键绑定上的操作。
#[derive(Default)]
struct BindingOutcome {
    remove: bool,
    start_recording: bool,
    /// 这条绑定正在录制时，热键框的位置。
    record_rect: Option<egui::Rect>,
}

/// 一条热键绑定。
fn binding_editor(
    ui: &mut egui::Ui,
    texts: &Texts,
    index: usize,
    binding: &mut BindingSettings,
    reasons: &[String],
    recording: Option<RecordingView>,
) -> BindingOutcome {
    let mut outcome = BindingOutcome::default();
    let mut remove = false;
    let mut frame = egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(8));
    if !reasons.is_empty() {
        frame = frame.stroke(egui::Stroke::new(1.5, ui.visuals().error_fg_color));
    }
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(hotkey_text(binding.hotkey, texts)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                remove = ui.button(texts.get("settings-binding-remove")).clicked();
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(texts.get("settings-binding-hotkey"));
            let mine = recording.filter(|r| r.binding == index);
            let label = match mine {
                Some(view) => recording_prompt(view.preview, texts),
                None => hotkey_text(binding.hotkey, texts),
            };
            let response = ui
                .add(
                    egui::Button::selectable(mine.is_some(), RichText::new(label).monospace())
                        .min_size(egui::vec2(240.0, 0.0)),
                )
                .on_hover_text(texts.get("settings-hotkey-record-tooltip"));
            if mine.is_some() {
                outcome.record_rect = Some(response.rect);
            } else if response.clicked() {
                outcome.start_recording = true;
            }
            if let Some(view) = mine {
                if view.unsupported {
                    error(ui, texts.get("settings-record-unsupported"));
                } else {
                    hint(ui, texts.get("settings-record-help"));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(texts.get("settings-binding-mode"));
            egui::ComboBox::from_id_salt(("mode", index))
                .selected_text(binding.mode.localized_label(texts))
                .show_ui(ui, |ui| {
                    for mode in CaptureMode::ALL {
                        ui.selectable_value(&mut binding.mode, mode, mode.localized_label(texts));
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(texts.get("settings-binding-actions"));
            egui::ComboBox::from_id_salt(("kind", index))
                .selected_text(binding.kind.localized_label(texts))
                .show_ui(ui, |ui| {
                    for kind in CaptureKind::ALL {
                        ui.selectable_value(&mut binding.kind, kind, kind.localized_label(texts));
                    }
                });
            // 录屏总是保存、OCR 不保存：自动保存只对截图类绑定和长截屏有意义。
            ui.add_enabled(
                binding.kind.supports_auto_save(),
                egui::Checkbox::new(
                    &mut binding.auto_save,
                    texts.get("settings-binding-auto-save"),
                ),
            );
        });
        for reason in reasons {
            error(ui, reason.clone());
        }
    });
    outcome.remove = remove;
    outcome
}

/// 热键框里的文字：热键本身，清空后显示“未设置”。
fn hotkey_text(hotkey: Option<Hotkey>, texts: &Texts) -> String {
    hotkey.map_or_else(|| texts.get("settings-hotkey-unset"), |h| h.to_string())
}

/// 录制中热键框里的文字：提示，加上正按住的修饰键。
fn recording_prompt(held: Modifiers, texts: &Texts) -> String {
    let mut text = String::new();
    for (on, name) in [
        (held.win, "Win+"),
        (held.ctrl, "Ctrl+"),
        (held.alt, "Alt+"),
        (held.shift, "Shift+"),
    ] {
        if on {
            text.push_str(name);
        }
    }
    if text.is_empty() {
        texts.get("settings-record-prompt")
    } else {
        text
    }
}

/// 绑定自己的问题已经显示在这条绑定里，不必再说是第几条。
fn binding_problem(problem: &SettingsProblem, texts: &Texts) -> String {
    match problem {
        SettingsProblem::TypingKeyWithoutModifier { .. } => {
            texts.get("settings-binding-needs-modifier")
        }
        other => other.localized(texts),
    }
}

/// 新增绑定时默认用的热键：还没被用过的 Ctrl+F1…F12，都用过了就用 Ctrl+Alt+F1…F12。
fn unused_hotkey(bindings: &[BindingSettings]) -> Hotkey {
    [Modifiers::CTRL, Modifiers::CTRL.with(Modifiers::ALT)]
        .into_iter()
        .flat_map(|modifiers| (1..=12).map(move |n| Hotkey::new(modifiers, Key::F(n))))
        .find(|hotkey| bindings.iter().all(|b| b.hotkey != Some(*hotkey)))
        .unwrap_or(Hotkey::new(Modifiers::CTRL, Key::F(1)))
}

const OUTPUT_FORMATS: [OutputFormat; 4] = [
    OutputFormat::Png,
    OutputFormat::Jpeg,
    OutputFormat::WebP,
    OutputFormat::Avif,
];

fn format_label(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Png => "PNG",
        OutputFormat::Jpeg => "JPEG",
        OutputFormat::WebP => "WebP",
        OutputFormat::Avif => "AVIF",
    }
}

/// 语言选项：具体的语言总用它自己的名字，方便看不懂当前语言的人找到自己的语言。
fn language_label(language: Language, texts: &Texts) -> String {
    match language {
        Language::Auto => texts.get("settings-language-auto"),
        Language::ZhCn => "简体中文".to_owned(),
        Language::En => "English".to_owned(),
    }
}

fn hint(ui: &mut egui::Ui, text: String) {
    ui.add(egui::Label::new(RichText::new(text).small().weak()).wrap());
}

fn warning(ui: &mut egui::Ui, text: String) {
    let color = ui.visuals().warn_fg_color;
    ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
}

fn error(ui: &mut egui::Ui, text: String) {
    let color = ui.visuals().error_fg_color;
    ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
}
