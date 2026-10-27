//! 测试用的整套假平台实现。
//!
//! 用法：先用 [`FakeDisplay`] 和 [`FakePlatform`] 的构造方法描述“桌面”，
//! 再用 [`FakePlatform::platform`] 生成交给 `bps-core` 的 [`Platform`]；
//! `FakePlatform` 本身保留一份共享状态，测试通过它模拟用户操作（移动鼠标、按热键、
//! 编辑配置文件、修改系统设置、选托盘菜单、关闭桌面上的窗口、在选区遮罩上框选或取消）
//! 并检查外部可见的结果（剪贴板内容、写入的文件、发出的通知、已注册的热键、配置文件内容、
//! 开机自启登记、遮罩当前画的场景和画布、录屏控制条、弹出过的“另存为”对话框、托盘菜单的文字、打开的独立窗口、
//! 检查更新发出的请求）。“另存为”对话框里用户的选择要预先用 [`FakePlatform::answer_save_as`] 安排好。
//!
//! 翻译用的 HTTP 传输同样不在 `Platform` 里：用 [`FakePlatform::http_transport`] 取得，
//! 回复用 [`FakePlatform::with_http_reply`] / [`FakePlatform::with_http_failure`] 按顺序安排，
//! [`FakePlatform::http_requests`] 查看发出过哪些请求。
//!
//! 结果面板子进程用的文字识别（OCR）不在 `Platform` 里：用 [`FakePlatform::text_recognizer`]
//! 取得，识别结果用 [`FakePlatform::with_recognized_text`] 预先设置，
//! [`FakePlatform::recognition_requests`] 查看识别过哪些图像。
//!
//! 遮罩上的操作（[`FakePlatform::select_region`]、[`FakePlatform::drag`]、
//! [`FakePlatform::press_key`]、[`FakePlatform::type_text`] 等）只生成 `PlatformEvent`，由测试交给常驻实例处理，
//! 和真实平台“窗口过程把输入放进事件队列”的方式一致。
//!
//! 标注文字的字形由 [`text_mask`] 按简单规则生成（方块代替字形），测试可以据此写出期望像素。
//!
//! CLI 一侧用 [`FakeResidentChannel`]：它把请求同步交给测试里的常驻实例。
//!
//! 录屏：单调时间只在测试调用 [`FakePlatform::advance_clock`] 时前进；节拍开始后，
//! [`FakePlatform::tick`] 生成平台事件循环会递交的节拍事件。假视频源的帧是此刻假显示器画面的
//! 副本，假编码器写出的视频用 [`decode::video`] 读回。系统声音用 [`FakePlatform::play_audio`] 播放，
//! 麦克风用 [`FakePlatform::speak`] 说话。
//!
//! ```
//! use bps_platform::{Point, Rect};
//! use bps_platform_fake::{FakeDisplay, FakePlatform};
//!
//! let fake = FakePlatform::new()
//!     .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
//!     .with_cursor_at(Point::new(10, 10));
//! let platform = fake.platform(); // 交给 ResidentInstance::new
//! # drop(platform);
//! assert!(fake.clipboard().is_empty());
//! ```

mod audio;
mod autostart;
mod background;
mod clipboard;
mod config_file;
pub mod decode;
mod file_dialog;
mod file_system;
mod frame_source;
mod hotkeys;
mod http;
mod notifier;
mod ocr;
mod overlay;
mod recording_toolbar;
mod region_sampler;
mod resident_channel;
mod text;
mod tray;
mod update;
mod video;
mod window_source;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bps_platform::{
    AudioChunk, AudioDevice, AutostartEntry, AutostartMethod, Backdrop, Clock, Hotkey, HttpRequest,
    HttpResponse, HttpTransport, Image, Key, LatestRelease, LocalDateTime, Modifiers, Notification,
    OverlayInput, OverlayKey, OverlayScene, Platform, PlatformError, PlatformEvent, Point,
    PointerButton, RecognitionLanguage, RecordingToolbarView, Rect, SaveAsOptions,
    StandaloneWindow, SystemLanguage, TextRecognizer, TrayMenuItem, VideoCodec,
};

pub use audio::{DEFAULT_MICROPHONE, FAKE_AUDIO_FORMAT};
pub use clipboard::ClipboardContent;
pub use config_file::FAKE_CONFIG_PATH;
pub use file_system::DEFAULT_PICTURES_DIR;
pub use frame_source::{FakeDisplay, desktop_pattern};
pub use resident_channel::FakeResidentChannel;
pub use text::text_mask;
pub use video::{FAKE_CURSOR_COLOR, draw_fake_cursor};
pub use window_source::{FakeWindow, window_pattern};

use audio::{AudioState, FakeAudioSource};
use autostart::{FakeAutostart, FakeElevation, FakeScreenshotTool, SystemState};
use background::{BackgroundState, FakeBackground};
use clipboard::FakeClipboard;
use config_file::{ConfigFileState, FakeConfigFile};
use file_dialog::{DialogState, FakeFileDialog};
use file_system::{FakeFileSystem, FsState};
use frame_source::{FakeFrameSource, FrameState};
use hotkeys::{FakeHotkeys, HotkeyState};
use http::{FakeHttpTransport, HttpState};
use notifier::FakeNotifier;
use ocr::{FakeTextRecognizer, OcrState};
use overlay::{FakeOverlay, OverlayState};
use recording_toolbar::FakeRecordingToolbar;
use region_sampler::{FakeRegionSampler, SamplerState};
use tray::FakeTrayMenu;
use update::{FakeUpdateService, UpdateState};
use video::{FakeTicker, FakeVideoEncoder, FakeVideoSource, VideoState};
use window_source::{FakeWindowSource, WindowSourceState};
use windows::{FakeWindowLauncher, WindowsState};

/// 假时钟的默认时间。
pub const DEFAULT_TIME: LocalDateTime = LocalDateTime::new(2000, 1, 1, 0, 0, 0);

/// 假平台默认的系统界面语言。
pub const DEFAULT_SYSTEM_LANGUAGE: &str = "zh-CN";

/// 假平台的共享状态。克隆得到的是同一份状态的另一个句柄。
#[derive(Clone)]
pub struct FakePlatform {
    frames: Arc<Mutex<FrameState>>,
    clipboard: Arc<Mutex<Option<ClipboardContent>>>,
    hotkeys: Arc<Mutex<HotkeyState>>,
    files: Arc<Mutex<FsState>>,
    notifications: Arc<Mutex<Vec<Notification>>>,
    time: Arc<Mutex<LocalDateTime>>,
    /// 单调时间。
    monotonic: Arc<Mutex<Duration>>,
    config_file: Arc<Mutex<ConfigFileState>>,
    tray_menu: Arc<Mutex<Vec<TrayMenuItem>>>,
    windows: Arc<Mutex<WindowsState>>,
    system_languages: Arc<Mutex<Vec<String>>>,
    updates: Arc<Mutex<UpdateState>>,
    system: Arc<Mutex<SystemState>>,
    overlay: Arc<Mutex<OverlayState>>,
    video: Arc<Mutex<VideoState>>,
    audio: Arc<Mutex<AudioState>>,
    /// 正显示着的录屏控制条。
    recording_toolbar: Arc<Mutex<Option<RecordingToolbarView>>>,
    dialogs: Arc<Mutex<DialogState>>,
    sampler: Arc<Mutex<SamplerState>>,
    window_source: Arc<Mutex<WindowSourceState>>,
    ocr: Arc<Mutex<OcrState>>,
    background: Arc<Mutex<BackgroundState>>,
    http: Arc<Mutex<HttpState>>,
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self {
            frames: Default::default(),
            clipboard: Default::default(),
            hotkeys: Default::default(),
            files: Default::default(),
            notifications: Default::default(),
            time: Arc::new(Mutex::new(DEFAULT_TIME)),
            monotonic: Default::default(),
            config_file: Default::default(),
            tray_menu: Default::default(),
            windows: Default::default(),
            system_languages: Arc::new(Mutex::new(vec![DEFAULT_SYSTEM_LANGUAGE.to_owned()])),
            updates: Default::default(),
            system: Default::default(),
            overlay: Default::default(),
            video: Default::default(),
            audio: Default::default(),
            recording_toolbar: Default::default(),
            dialogs: Default::default(),
            sampler: Default::default(),
            window_source: Default::default(),
            ocr: Default::default(),
            background: Default::default(),
            http: Default::default(),
        }
    }
}

impl FakePlatform {
    /// 没有任何显示器、鼠标位于 (0, 0)、剪贴板为空、只有一个空的“图片”文件夹
    /// （[`DEFAULT_PICTURES_DIR`]）、时间为 [`DEFAULT_TIME`]、配置文件不存在、
    /// 没有开机自启登记、以普通权限运行、系统截图工具不占用 PrtSc、系统界面语言为
    /// [`DEFAULT_SYSTEM_LANGUAGE`] 的平台。
    pub fn new() -> Self {
        Self::default()
    }

    /// 接入一块显示器；按添加顺序分配 `DisplayId`。
    pub fn with_display(self, display: FakeDisplay) -> Self {
        self.frames.lock().unwrap().displays.push(display);
        self
    }

    /// 在桌面上打开一个顶层窗口；后添加的窗口在上层（像依次打开的窗口）。
    pub fn with_window(self, window: FakeWindow) -> Self {
        self.window_source
            .lock()
            .unwrap()
            .windows
            .push(Some(window));
        self
    }

    /// 设置鼠标的初始位置（虚拟桌面物理像素）。
    pub fn with_cursor_at(self, position: Point) -> Self {
        self.move_cursor(position);
        self
    }

    /// 设置时钟的当前时间。
    pub fn with_time(self, time: LocalDateTime) -> Self {
        self.set_time(time);
        self
    }

    /// 把“图片”文件夹换到 `path`（会创建它）。
    pub fn with_pictures_dir(self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut files = self.files.lock().unwrap();
        files.add_dir(&path);
        files.pictures_dir = path;
        drop(files);
        self
    }

    /// 预先创建一个目录（及其上级目录）。
    pub fn with_dir(self, path: impl AsRef<Path>) -> Self {
        self.files.lock().unwrap().add_dir(path.as_ref());
        self
    }

    /// 预先放入一个文件（自动创建上级目录）。
    pub fn with_file(self, path: impl AsRef<Path>, contents: impl Into<Vec<u8>>) -> Self {
        self.files
            .lock()
            .unwrap()
            .add_file(path.as_ref(), contents.into());
        self
    }

    /// 创建一个只读目录：在它里面创建目录或文件都会失败。
    pub fn with_read_only_dir(self, path: impl AsRef<Path>) -> Self {
        self.files.lock().unwrap().add_read_only_dir(path.as_ref());
        self
    }

    /// 配置文件已经存在，内容是 `text`。
    pub fn with_config_file(self, text: &str) -> Self {
        self.config_file.lock().unwrap().text = Some(text.to_owned());
        self
    }

    /// `hotkey` 已被其他程序占用，注册它会失败。
    pub fn with_hotkey_taken_by_other_program(self, hotkey: Hotkey) -> Self {
        self.hotkeys.lock().unwrap().occupied.push(hotkey);
        self
    }

    /// 已经登记了 `method` 方式的开机自启，指向本程序。
    pub fn with_autostart_entry(self, method: AutostartMethod) -> Self {
        self.add_autostart_entry(method, true)
    }

    /// 已经登记了 `method` 方式的开机自启，但指向别处的另一份 BetterPrtSc。
    pub fn with_autostart_entry_of_other_program(self, method: AutostartMethod) -> Self {
        self.add_autostart_entry(method, false)
    }

    fn add_autostart_entry(self, method: AutostartMethod, this_program: bool) -> Self {
        let mut system = self.system.lock().unwrap();
        system.entries.retain(|e| e.method != method);
        system.entries.push(AutostartEntry {
            method,
            this_program,
        });
        drop(system);
        self
    }

    /// 登记和移除开机自启都会以 `message` 失败（例如被安全软件拦截）。
    pub fn with_autostart_failing(self, message: &str) -> Self {
        self.system.lock().unwrap().autostart_error = Some(message.to_owned());
        self
    }

    /// 本进程以管理员权限运行。
    pub fn as_administrator(self) -> Self {
        self.system.lock().unwrap().elevated = true;
        self
    }

    /// 以管理员身份重新启动时，用户会在 UAC 提示里点“否”。
    pub fn with_administrator_restart_declined(self) -> Self {
        self.system.lock().unwrap().elevation_declined = true;
        self
    }

    /// 系统设置把 PrtSc 键交给了系统截图工具：注册不带修饰键的 PrtSc 会失败。
    pub fn with_print_screen_key_taken_by_system_screenshot_tool(self) -> Self {
        self.system
            .lock()
            .unwrap()
            .screenshot_tool_takes_print_screen = true;
        self.hotkeys.lock().unwrap().occupied.push(PRINT_SCREEN);
        self
    }

    /// 模拟用户在系统设置里关闭“使用 Print Screen 键打开屏幕截图”：
    /// 正在监视时返回平台事件循环会递交的事件。
    pub fn turn_off_print_screen_key_for_system_screenshot_tool(&self) -> Option<PlatformEvent> {
        self.hotkeys
            .lock()
            .unwrap()
            .occupied
            .retain(|h| *h != PRINT_SCREEN);
        let mut system = self.system.lock().unwrap();
        system.screenshot_tool_takes_print_screen = false;
        system
            .screenshot_tool_watched
            .then_some(PlatformEvent::PrintScreenKeySettingChanged)
    }

    /// 模拟用户修改了同一组系统设置里的其他项（PrtSc 的占用不变）：
    /// 正在监视时返回平台事件循环会递交的事件。
    pub fn change_other_keyboard_setting(&self) -> Option<PlatformEvent> {
        self.system
            .lock()
            .unwrap()
            .screenshot_tool_watched
            .then_some(PlatformEvent::PrintScreenKeySettingChanged)
    }

    /// 系统的界面语言（按优先级排列），例如 `["en-US"]`。
    pub fn with_system_languages(self, languages: &[&str]) -> Self {
        *self.system_languages.lock().unwrap() =
            languages.iter().map(|l| (*l).to_owned()).collect();
        self
    }

    /// 让打开独立窗口失败（例如找不到程序文件）。
    pub fn with_window_launch_failing(self) -> Self {
        self.windows.lock().unwrap().fails = true;
        self
    }

    /// 模拟用户移动鼠标。
    pub fn move_cursor(&self, position: Point) {
        self.frames.lock().unwrap().cursor = position;
    }

    /// 模拟屏幕画面变化（例如鼠标移开后悬停菜单消失）：第 `index` 块显示器（按添加顺序，从 0 开始）
    /// 改为显示 `image`，尺寸必须等于它的物理像素尺寸。
    pub fn change_display_image(&self, index: usize, image: Image) {
        let mut frames = self.frames.lock().unwrap();
        let display = &mut frames.displays[index];
        *display = display.clone().with_image(image);
    }

    /// 模拟用户关闭第 `index` 个窗口（按添加顺序，从 0 开始）：之后的窗口布局里没有它，
    /// 截取它会失败。
    /// 把第 `index` 个添加的窗口改成 `width` × `height`（左上角不动，内容换成新尺寸的
    /// [`window_pattern`]）。
    pub fn resize_window(&self, index: usize, width: u32, height: u32) {
        self.window_source
            .lock()
            .unwrap()
            .resize(index, width, height);
    }

    pub fn close_window(&self, index: usize) {
        self.window_source.lock().unwrap().windows[index] = None;
    }

    /// 模拟时间流逝。
    pub fn set_time(&self, time: LocalDateTime) {
        *self.time.lock().unwrap() = time;
    }

    /// 让单调时间前进 `duration`（本地时间不受影响）。
    pub fn advance_clock(&self, duration: Duration) {
        *self.monotonic.lock().unwrap() += duration;
    }

    /// 节拍到了：节拍正在运行时返回平台事件循环会递交的事件，否则返回 `None`。
    /// 节拍不会自己发生，测试按需要调用（真实平台上节拍也可能晚到或被合并）。
    pub fn tick(&self) -> Option<PlatformEvent> {
        self.ticker_period().map(|_| PlatformEvent::Tick)
    }

    /// 正在运行的节拍的周期；没有节拍时返回 `None`。
    pub fn ticker_period(&self) -> Option<Duration> {
        self.video.lock().unwrap().ticker
    }

    /// 视频源是否正在捕获画面（有还没关闭的视频流）。
    pub fn capturing_video(&self) -> bool {
        self.video.lock().unwrap().open_streams > 0
    }

    /// 视频源打开后迟迟收不到第一帧，直到调用 [`deliver_video_frames`](Self::deliver_video_frames)。
    pub fn with_video_frames_delayed(self) -> Self {
        self.video.lock().unwrap().pending = true;
        self
    }

    /// 视频源开始收到画面。
    pub fn deliver_video_frames(&self) {
        self.video.lock().unwrap().pending = false;
    }

    /// 模拟视频源捕获的目标消失（例如显示器被拔掉）：之后视频流不再有新的帧。
    pub fn end_video_source(&self) {
        self.video.lock().unwrap().ended = true;
    }

    /// 视频编码器开始编码时会以 `message` 失败（例如没有可用的编码器）。
    pub fn with_video_encoder_failing(self, message: &str) -> Self {
        self.video.lock().unwrap().encoder_error = Some(message.to_owned());
        self
    }

    /// 没有可用的音频输出设备：打开系统声音会以 `reason` 失败。
    pub fn without_audio_device(self, reason: &str) -> Self {
        self.audio.lock().unwrap().system.unavailable = Some(reason.to_owned());
        self
    }

    /// 系统里的麦克风换成 `devices`（标识、名字），第一个是系统默认设备；空列表表示没有麦克风。
    pub fn with_microphones(self, devices: &[(&str, &str)]) -> Self {
        self.audio.lock().unwrap().microphones = devices
            .iter()
            .map(|&(id, name)| AudioDevice {
                id: id.to_owned(),
                name: name.to_owned(),
            })
            .collect();
        self
    }

    /// 麦克风打不开：打开任何麦克风都会以 `reason` 失败。
    pub fn with_microphone_failing(self, reason: &str) -> Self {
        self.audio.lock().unwrap().microphone.unavailable = Some(reason.to_owned());
        self
    }

    /// 视频编码器不支持 `codec`：用它开始编码会以 `message` 失败。
    pub fn without_video_codec(self, codec: VideoCodec, message: &str) -> Self {
        self.video
            .lock()
            .unwrap()
            .unsupported_codecs
            .push((codec, message.to_owned()));
        self
    }

    /// 视频编码器开始编码带音轨的视频时会以 `message` 失败（不带音轨时正常）。
    pub fn with_audio_encoder_failing(self, message: &str) -> Self {
        self.video.lock().unwrap().audio_encoder_error = Some(message.to_owned());
        self
    }

    /// 模拟音频输出设备中途失效：之后读取系统声音都以 `reason` 失败。
    pub fn fail_audio_reads(&self, reason: &str) {
        self.audio.lock().unwrap().system.read_error = Some(reason.to_owned());
    }

    /// 模拟麦克风中途被拔掉：之后读取麦克风都以 `reason` 失败。
    pub fn fail_microphone_reads(&self, reason: &str) {
        self.audio.lock().unwrap().microphone.read_error = Some(reason.to_owned());
    }

    /// 是否正在采集系统声音（有还没关闭的音频流）。
    pub fn capturing_audio(&self) -> bool {
        self.audio.lock().unwrap().system.open_streams > 0
    }

    /// 是否正在采集麦克风。
    pub fn capturing_microphone(&self) -> bool {
        self.audio.lock().unwrap().microphone.open_streams > 0
    }

    /// 最近一次成功打开的麦克风：`Some(None)` 是系统默认设备，`None` 表示从没打开过。
    pub fn opened_microphone(&self) -> Option<Option<String>> {
        self.audio.lock().unwrap().opened_microphone.clone()
    }

    /// 系统从此刻（单调时间）起播放一段 `duration` 长、每个样本都是 `level` 的声音
    /// （[`FAKE_AUDIO_FORMAT`]：每毫秒一个样本）。正在采集时，音频流下一次读取时交出它。
    pub fn play_audio(&self, level: f32, duration: Duration) {
        self.play_audio_since(level, Duration::ZERO, duration);
    }

    /// 同 [`play_audio`](Self::play_audio)，但声音从 `ago` 之前就开始了：平台晚交出的声音
    /// （例如暂停前采集、暂停后才读到）。
    pub fn play_audio_since(&self, level: f32, ago: Duration, duration: Duration) {
        let captured_at = self.monotonic.lock().unwrap().saturating_sub(ago);
        let mut audio = self.audio.lock().unwrap();
        push_sound(&mut audio.system, captured_at, level, duration);
    }

    /// 用户从此刻起对着麦克风说了一段 `duration` 长、每个样本都是 `level` 的话。
    /// 正在采集麦克风时，麦克风的音频流下一次读取时交出它。
    pub fn speak(&self, level: f32, duration: Duration) {
        let captured_at = *self.monotonic.lock().unwrap();
        let mut audio = self.audio.lock().unwrap();
        push_sound(&mut audio.microphone, captured_at, level, duration);
    }

    /// 生成交给 `bps-core` 的平台能力集合，它与本对象共享状态。
    pub fn platform(&self) -> Platform {
        Platform {
            frame_source: Box::new(FakeFrameSource(self.frames.clone())),
            clipboard: Box::new(FakeClipboard(self.clipboard.clone())),
            hotkeys: Box::new(FakeHotkeys(self.hotkeys.clone())),
            file_system: Box::new(FakeFileSystem(self.files.clone())),
            notifier: Box::new(FakeNotifier(self.notifications.clone())),
            clock: Box::new(FakeClock {
                time: self.time.clone(),
                monotonic: self.monotonic.clone(),
            }),
            config_file: Box::new(FakeConfigFile {
                state: self.config_file.clone(),
                path: self.config_file_path(),
            }),
            tray_menu: Box::new(FakeTrayMenu(self.tray_menu.clone())),
            windows: Box::new(FakeWindowLauncher(self.windows.clone())),
            system_language: Box::new(FakeSystemLanguage(self.system_languages.clone())),
            updates: Box::new(FakeUpdateService(self.updates.clone())),
            background: Box::new(FakeBackground(self.background.clone())),
            autostart: Box::new(FakeAutostart(self.system.clone())),
            elevation: Box::new(FakeElevation(self.system.clone())),
            system_screenshot_tool: Box::new(FakeScreenshotTool(self.system.clone())),
            overlay: Box::new(FakeOverlay {
                frames: self.frames.clone(),
                state: self.overlay.clone(),
            }),
            video_source: Box::new(FakeVideoSource {
                windows: self.window_source.clone(),
                frames: self.frames.clone(),
                state: self.video.clone(),
            }),
            video_encoder: Box::new(FakeVideoEncoder {
                files: self.files.clone(),
                state: self.video.clone(),
            }),
            audio_source: Box::new(FakeAudioSource(self.audio.clone())),
            ticker: Box::new(FakeTicker(self.video.clone())),
            recording_toolbar: Box::new(FakeRecordingToolbar(self.recording_toolbar.clone())),
            file_dialog: Box::new(FakeFileDialog(self.dialogs.clone())),
            window_source: Box::new(FakeWindowSource(self.window_source.clone())),
            region_sampler: Box::new(FakeRegionSampler(self.sampler.clone())),
        }
    }

    /// 安排用户在下一次弹出的“另存为”对话框里的选择：选定 `path`，或取消（`None`）。
    /// 可以连续安排多次，按弹出顺序使用；弹出时没有安排好的选择会让测试 panic。
    pub fn answer_save_as(&self, path: Option<PathBuf>) {
        self.dialogs.lock().unwrap().answers.push_back(path);
    }

    /// 至今弹出过的“另存为”对话框，按弹出顺序排列。
    pub fn save_as_dialogs(&self) -> Vec<SaveAsOptions> {
        self.dialogs.lock().unwrap().shown.clone()
    }

    /// 当前的开机自启登记：普通方式在前。
    pub fn autostart_entries(&self) -> Vec<AutostartEntry> {
        self.system.lock().unwrap().sorted_entries()
    }

    /// 至今以管理员身份重新启动的次数（不论用户是否同意）。
    pub fn administrator_restarts(&self) -> usize {
        self.system.lock().unwrap().administrator_restarts
    }

    /// 配置文件的位置（[`FAKE_CONFIG_PATH`]）。
    pub fn config_file_path(&self) -> PathBuf {
        PathBuf::from(FAKE_CONFIG_PATH)
    }

    /// 配置文件当前的内容；文件不存在时返回 `None`。
    pub fn config_file_text(&self) -> Option<String> {
        self.config_file.lock().unwrap().text.clone()
    }

    /// 模拟用户在外部编辑器里修改配置文件：正在监视时返回平台事件循环会递交的事件。
    pub fn edit_config_file(&self, text: &str) -> Option<PlatformEvent> {
        let mut state = self.config_file.lock().unwrap();
        state.text = Some(text.to_owned());
        state.watched.then_some(PlatformEvent::ConfigFileChanged)
    }

    /// 模拟用户删除配置文件：正在监视时返回平台事件循环会递交的事件。
    pub fn delete_config_file(&self) -> Option<PlatformEvent> {
        let mut state = self.config_file.lock().unwrap();
        state.text = None;
        state.watched.then_some(PlatformEvent::ConfigFileChanged)
    }

    /// 剪贴板当前内容的快照。
    pub fn clipboard(&self) -> ClipboardContent {
        self.clipboard.lock().unwrap().clone().unwrap_or_default()
    }

    /// “图片”文件夹的路径。
    pub fn pictures_dir(&self) -> PathBuf {
        self.files.lock().unwrap().pictures_dir.clone()
    }

    /// 文件 `path` 的内容；文件不存在时返回 `None`。
    pub fn file(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.files.lock().unwrap().file(path.as_ref())
    }

    /// 所有文件的路径，按路径排序。
    pub fn files(&self) -> Vec<PathBuf> {
        self.files.lock().unwrap().file_paths()
    }

    /// 至今发出的所有通知，按发出顺序排列。
    pub fn notifications(&self) -> Vec<Notification> {
        self.notifications.lock().unwrap().clone()
    }

    /// 模拟用户按下热键：已注册时返回平台事件循环会递交的事件，未注册时返回 `None`。
    pub fn press(&self, hotkey: Hotkey) -> Option<PlatformEvent> {
        self.hotkeys
            .lock()
            .unwrap()
            .id_of(hotkey)
            .map(PlatformEvent::HotkeyPressed)
    }

    /// 当前已注册的全局热键。
    pub fn registered_hotkeys(&self) -> Vec<Hotkey> {
        self.hotkeys.lock().unwrap().hotkeys()
    }

    /// 托盘菜单当前各项的文字，按显示顺序排列。
    pub fn tray_menu_labels(&self) -> Vec<String> {
        let items = self.tray_menu.lock().unwrap();
        items.iter().map(|item| item.label.clone()).collect()
    }

    /// 模拟用户在托盘菜单里选中文字为 `label` 的一项：返回平台事件循环会递交的事件，
    /// 菜单里没有这一项时返回 `None`。
    pub fn choose_tray_menu_item(&self, label: &str) -> Option<PlatformEvent> {
        let items = self.tray_menu.lock().unwrap();
        items
            .iter()
            .find(|item| item.label == label)
            .map(|item| PlatformEvent::TrayMenuItemSelected(item.id))
    }

    /// 系统已为识别语言 `tag`（显示名 `name`）安装了 OCR 语言包。
    pub fn with_recognition_language(self, tag: &str, name: &str) -> Self {
        self.install_recognition_language(tag, name);
        self
    }

    /// 模拟用户在系统设置里为 `tag` 安装了 OCR 语言包。
    pub fn install_recognition_language(&self, tag: &str, name: &str) {
        self.ocr
            .lock()
            .unwrap()
            .languages
            .push(RecognitionLanguage {
                tag: tag.to_owned(),
                name: name.to_owned(),
            });
    }

    /// 用户的语言首选项里第一个已安装 OCR 语言包的语言是 `tag`（跟随系统的默认识别语言）。
    pub fn with_default_recognition_language(self, tag: &str) -> Self {
        self.ocr.lock().unwrap().default_language = Some(tag.to_owned());
        self
    }

    /// 用识别语言 `language` 识别任何图像，都得到 `lines`（每行是按阅读顺序排列的词）。
    pub fn with_recognized_text(self, language: &str, lines: &[&[&str]]) -> Self {
        let lines = lines
            .iter()
            .map(|words| words.iter().map(|w| (*w).to_owned()).collect())
            .collect();
        self.ocr
            .lock()
            .unwrap()
            .results
            .insert(language.to_owned(), lines);
        self
    }

    /// 识别文字会以 `message` 失败。
    pub fn with_text_recognition_failing(self, message: &str) -> Self {
        self.set_text_recognition_failing(Some(message));
        self
    }

    /// 让之后的识别以 `message` 失败；`None` 恢复正常。
    pub fn set_text_recognition_failing(&self, message: Option<&str>) {
        self.ocr.lock().unwrap().failure = message.map(str::to_owned);
    }

    /// 结果面板子进程用的文字识别（OCR）能力。
    pub fn text_recognizer(&self) -> Box<dyn TextRecognizer + Send + Sync> {
        Box::new(FakeTextRecognizer(self.ocr.clone()))
    }

    /// 文字识别至今的每次识别：被识别的图像和所用的识别语言，按先后排列。
    pub fn recognition_requests(&self) -> Vec<(Image, String)> {
        self.ocr.lock().unwrap().requests.clone()
    }

    /// 安排下一个 HTTP 请求得到状态码 `status`、正文 `body` 的回复（按安排的顺序依次返回）。
    pub fn with_http_reply(self, status: u16, body: &str) -> Self {
        self.http
            .lock()
            .unwrap()
            .replies
            .push_back(Ok(HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            }));
        self
    }

    /// 安排下一个 HTTP 请求因网络问题失败，原因为 `message`。
    pub fn with_http_failure(self, message: &str) -> Self {
        self.http
            .lock()
            .unwrap()
            .replies
            .push_back(Err(message.to_owned()));
        self
    }

    /// 翻译用的 HTTP 传输能力。
    pub fn http_transport(&self) -> Box<dyn HttpTransport + Send + Sync> {
        Box::new(FakeHttpTransport(self.http.clone()))
    }

    /// 已提交、还没有运行的后台任务数。
    pub fn pending_background_jobs(&self) -> usize {
        self.background.lock().unwrap().pending.len()
    }

    /// 让后台线程把已提交的任务按提交顺序全部运行完：返回平台事件循环会递交的完成事件
    /// （每个任务一个，要交给常驻实例）。
    pub fn run_background_jobs(&self) -> Vec<PlatformEvent> {
        let mut events = Vec::new();
        loop {
            // 任务运行时不持有锁。
            let Some(job) = self.background.lock().unwrap().pending.pop_front() else {
                return events;
            };
            let result = job();
            self.background.lock().unwrap().finished.push_back(result);
            events.push(PlatformEvent::BackgroundJobDone);
        }
    }

    /// 至今发出的每个 HTTP 请求，按先后排列。
    pub fn http_requests(&self) -> Vec<HttpRequest> {
        self.http.lock().unwrap().requests.clone()
    }

    /// 至今打开过的独立窗口，按打开顺序排列。
    pub fn opened_windows(&self) -> Vec<StandaloneWindow> {
        self.windows.lock().unwrap().opened.clone()
    }

    /// 检查更新至今发出的查询（每一项是被查询的仓库），即发出过的全部网络请求。
    pub fn update_check_requests(&self) -> Vec<String> {
        self.updates.lock().unwrap().requests.clone()
    }

    /// 让最早一个还没返回的检查更新查询以 `result` 结束：返回平台事件循环会递交的事件；
    /// 没有正在进行的查询时返回 `None`。
    pub fn finish_update_check(
        &self,
        result: Result<Option<LatestRelease>, PlatformError>,
    ) -> Option<PlatformEvent> {
        let mut state = self.updates.lock().unwrap();
        if state.pending == 0 {
            return None;
        }
        state.pending -= 1;
        Some(PlatformEvent::LatestReleaseFetched(result))
    }

    /// 选区遮罩是否正显示在屏幕上。
    pub fn overlay_visible(&self) -> bool {
        self.overlay_scene().is_some()
    }

    /// 遮罩上当前画着的场景；遮罩没有显示时返回 `None`。
    pub fn overlay_scene(&self) -> Option<OverlayScene> {
        self.overlay.lock().unwrap().visible_scene().cloned()
    }

    /// 正显示着的遮罩底下是什么（冻结帧或实时画面）；遮罩没有显示时返回 `None`。
    pub fn overlay_backdrop(&self) -> Option<Backdrop> {
        self.overlay.lock().unwrap().visible_backdrop()
    }

    /// 遮罩上用户正看到的画布：位置和内容；没有画布或遮罩没有显示时返回 `None`。
    pub fn overlay_canvas(&self) -> Option<(Rect, Image)> {
        self.overlay.lock().unwrap().visible_canvas().cloned()
    }

    /// 在遮罩上单击写着 `text` 的地方（[`OverlayItem::Text`](bps_platform::OverlayItem::Text)
    /// 所在矩形的中心），例如工具条上的按钮。遮罩上没有这段文字时 panic。
    pub fn click_text(&self, text: &str) -> Vec<PlatformEvent> {
        let scene = self.overlay_scene().expect("选区遮罩没有显示");
        let rect = scene
            .text_rect(text)
            .unwrap_or_else(|| panic!("遮罩上没有“{text}”"));
        let center = Point::new(
            rect.x + rect.width as i32 / 2,
            rect.y + rect.height as i32 / 2,
        );
        self.click(PointerButton::Left, center)
    }

    /// 模拟用户在遮罩上的一次输入，返回平台事件循环会递交的事件。
    ///
    /// 遮罩没有显示时用户无法操作它，直接 panic。
    pub fn overlay_input(&self, input: OverlayInput) -> PlatformEvent {
        assert!(
            self.overlay_visible(),
            "选区遮罩没有显示，用户无法在上面操作"
        );
        let position = match input {
            OverlayInput::PointerMoved { position, .. }
            | OverlayInput::PointerPressed { position, .. }
            | OverlayInput::PointerReleased { position, .. } => Some(position),
            OverlayInput::KeyPressed { .. }
            | OverlayInput::Character(_)
            | OverlayInput::Interrupted => None,
        };
        if let (Some(position), Some(area)) = (position, self.overlay_pass_through()) {
            assert!(
                !area.contains(position),
                "{position:?} 在透过遮罩的范围 {area:?} 里，鼠标操作会交给底下的窗口"
            );
        }
        PlatformEvent::OverlayInput(input)
    }

    /// 遮罩上透过遮罩的范围（用户在这里看到实时桌面、鼠标操作交给底下的窗口）；
    /// 遮罩没有打开或没有透过的范围时返回 `None`。
    pub fn overlay_pass_through(&self) -> Option<Rect> {
        self.overlay.lock().unwrap().visible_pass_through()
    }

    /// 正在采样的区域；没有在采样时返回 `None`。
    pub fn sampling_region(&self) -> Option<Rect> {
        self.sampler.lock().unwrap().region
    }

    /// 至今开始采样过的区域，按开始顺序排列。
    pub fn sampled_regions(&self) -> Vec<Rect> {
        self.sampler.lock().unwrap().started.clone()
    }

    /// 模拟区域里的画面有了变化、平台采到一帧：正在采样时截下此刻桌面上的那块区域（各显示器
    /// 当前的画面，用 [`change_display_image`](Self::change_display_image) 改变），返回平台
    /// 事件循环会递交的事件；没有在采样时返回 `None`。
    pub fn sample_region(&self) -> Option<PlatformEvent> {
        let frames = self.frames.lock().unwrap();
        self.sampler
            .lock()
            .unwrap()
            .sample(&frames)
            .then_some(PlatformEvent::RegionSampled)
    }

    /// 模拟采样出错（例如显示器被拔掉）：正在采样时返回平台事件循环会递交的事件。
    pub fn fail_sampling(&self, message: &str) -> Option<PlatformEvent> {
        let mut sampler = self.sampler.lock().unwrap();
        sampler.region?;
        sampler.pending = Some(Err(PlatformError::new(message)));
        Some(PlatformEvent::RegionSampled)
    }

    /// 在遮罩上按住左键从 `from` 拖到 `to` 再松开（不按修饰键）。
    pub fn drag(&self, from: Point, to: Point) -> Vec<PlatformEvent> {
        let middle = Point::new(from.x + (to.x - from.x) / 2, from.y + (to.y - from.y) / 2);
        vec![
            self.overlay_input(pressed(PointerButton::Left, from, false)),
            self.overlay_input(moved(middle)),
            self.overlay_input(moved(to)),
            self.overlay_input(released(PointerButton::Left, to)),
        ]
    }

    /// 在遮罩上单击（按下再松开，不移动）。
    pub fn click(&self, button: PointerButton, at: Point) -> Vec<PlatformEvent> {
        vec![
            self.overlay_input(pressed(button, at, false)),
            self.overlay_input(released(button, at)),
        ]
    }

    /// 在遮罩上双击左键。
    pub fn double_click(&self, at: Point) -> Vec<PlatformEvent> {
        vec![
            self.overlay_input(pressed(PointerButton::Left, at, false)),
            self.overlay_input(released(PointerButton::Left, at)),
            self.overlay_input(pressed(PointerButton::Left, at, true)),
            self.overlay_input(released(PointerButton::Left, at)),
        ]
    }

    /// 在遮罩上按一个键（不按修饰键）。
    pub fn press_key(&self, key: OverlayKey) -> PlatformEvent {
        self.overlay_input(OverlayInput::KeyPressed {
            key,
            modifiers: Modifiers::NONE,
        })
    }

    /// 在遮罩上按住 `modifiers` 按一个键（例如 Ctrl+Z）。
    pub fn press_key_with(&self, key: OverlayKey, modifiers: Modifiers) -> PlatformEvent {
        self.overlay_input(OverlayInput::KeyPressed { key, modifiers })
    }

    /// 在遮罩上打字：`text` 的每个字符依次以 [`OverlayInput::Character`] 递交，包括用输入法
    /// 输入、确认后的中文（平台逐字递交）。只能含可打印字符，换行用 [`press_key`](Self::press_key)
    /// 按 Enter。
    pub fn type_text(&self, text: &str) -> Vec<PlatformEvent> {
        text.chars()
            .map(|c| {
                assert!(!c.is_control(), "type_text 只能输入可打印字符");
                self.overlay_input(OverlayInput::Character(c))
            })
            .collect()
    }

    /// 正显示着的录屏控制条（和区域边框）；没有显示时返回 `None`。
    pub fn recording_toolbar(&self) -> Option<RecordingToolbarView> {
        self.recording_toolbar.lock().unwrap().clone()
    }

    /// 模拟用户在录屏控制条上的一次输入，返回平台事件循环会递交的事件。
    ///
    /// 控制条没有显示时用户无法操作它，直接 panic。
    pub fn recording_toolbar_input(&self, input: OverlayInput) -> PlatformEvent {
        assert!(
            self.recording_toolbar().is_some(),
            "录屏控制条没有显示，用户无法在上面操作"
        );
        PlatformEvent::OverlayInput(input)
    }

    /// 在录屏控制条上单击写着 `text` 的按钮（文字所在矩形的中心）。控制条上没有这段文字时 panic。
    pub fn click_recording_toolbar(&self, text: &str) -> Vec<PlatformEvent> {
        let view = self.recording_toolbar().expect("录屏控制条没有显示");
        let rect = view
            .text_rect(text)
            .unwrap_or_else(|| panic!("录屏控制条上没有“{text}”：{:?}", view.texts()));
        let center = Point::new(
            rect.x + rect.width as i32 / 2,
            rect.y + rect.height as i32 / 2,
        );
        vec![
            self.recording_toolbar_input(pressed(PointerButton::Left, center, false)),
            self.recording_toolbar_input(released(PointerButton::Left, center)),
        ]
    }

    /// 用户框选 `region` 并确定：从它左上角的像素拖到右下角的像素，再按 Enter。
    pub fn select_region(&self, region: Rect) -> Vec<PlatformEvent> {
        let last = Point::new(
            region.x + region.width as i32 - 1,
            region.y + region.height as i32 - 1,
        );
        let mut events = self.drag(Point::new(region.x, region.y), last);
        events.push(self.press_key(OverlayKey::Enter));
        events
    }
}

fn pressed(button: PointerButton, position: Point, double_click: bool) -> OverlayInput {
    OverlayInput::PointerPressed {
        button,
        position,
        modifiers: Modifiers::NONE,
        double_click,
    }
}

fn moved(position: Point) -> OverlayInput {
    OverlayInput::PointerMoved {
        position,
        modifiers: Modifiers::NONE,
    }
}

fn released(button: PointerButton, position: Point) -> OverlayInput {
    OverlayInput::PointerReleased {
        button,
        position,
        modifiers: Modifiers::NONE,
    }
}

struct FakeSystemLanguage(Arc<Mutex<Vec<String>>>);

impl SystemLanguage for FakeSystemLanguage {
    fn preferred_ui_languages(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// 不带修饰键的 PrtSc。
const PRINT_SCREEN: Hotkey = Hotkey::new(Modifiers::NONE, Key::PrintScreen);

struct FakeClock {
    time: Arc<Mutex<LocalDateTime>>,
    monotonic: Arc<Mutex<Duration>>,
}

impl Clock for FakeClock {
    fn now(&self) -> LocalDateTime {
        *self.time.lock().unwrap()
    }

    fn monotonic(&self) -> Duration {
        *self.monotonic.lock().unwrap()
    }
}

/// 正在采集 `input` 时，把一段声音交给它的音频流。
fn push_sound(
    input: &mut audio::InputState,
    captured_at: Duration,
    level: f32,
    duration: Duration,
) {
    if input.open_streams > 0 {
        input.pending.push(AudioChunk {
            captured_at,
            samples: vec![level; duration.as_millis() as usize],
        });
    }
}
