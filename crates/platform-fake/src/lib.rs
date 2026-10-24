//! 测试用的整套假平台实现。
//!
//! 用法：先用 [`FakeDisplay`] 和 [`FakePlatform`] 的构造方法描述“桌面”，
//! 再用 [`FakePlatform::platform`] 生成交给 `bps-core` 的 [`Platform`]；
//! `FakePlatform` 本身保留一份共享状态，测试通过它模拟用户操作（移动鼠标、按热键、
//! 编辑配置文件）并检查外部可见的结果（剪贴板内容、写入的文件、发出的通知、
//! 已注册的热键、配置文件内容）。
//!
//! CLI 一侧用 [`FakeResidentChannel`]：它把请求同步交给测试里的常驻实例。
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

mod clipboard;
mod config_file;
pub mod decode;
mod file_system;
mod frame_source;
mod hotkeys;
mod notifier;
mod resident_channel;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bps_platform::{Clock, Hotkey, LocalDateTime, Notification, Platform, PlatformEvent, Point};

pub use clipboard::ClipboardContent;
pub use config_file::FAKE_CONFIG_PATH;
pub use file_system::DEFAULT_PICTURES_DIR;
pub use frame_source::{FakeDisplay, desktop_pattern};
pub use resident_channel::FakeResidentChannel;

use clipboard::FakeClipboard;
use config_file::{ConfigFileState, FakeConfigFile};
use file_system::{FakeFileSystem, FsState};
use frame_source::{FakeFrameSource, FrameState};
use hotkeys::{FakeHotkeys, HotkeyState};
use notifier::FakeNotifier;

/// 假时钟的默认时间。
pub const DEFAULT_TIME: LocalDateTime = LocalDateTime::new(2000, 1, 1, 0, 0, 0);

/// 假平台的共享状态。克隆得到的是同一份状态的另一个句柄。
#[derive(Clone)]
pub struct FakePlatform {
    frames: Arc<Mutex<FrameState>>,
    clipboard: Arc<Mutex<Option<ClipboardContent>>>,
    hotkeys: Arc<Mutex<HotkeyState>>,
    files: Arc<Mutex<FsState>>,
    notifications: Arc<Mutex<Vec<Notification>>>,
    time: Arc<Mutex<LocalDateTime>>,
    config_file: Arc<Mutex<ConfigFileState>>,
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
            config_file: Default::default(),
        }
    }
}

impl FakePlatform {
    /// 没有任何显示器、鼠标位于 (0, 0)、剪贴板为空、只有一个空的“图片”文件夹
    /// （[`DEFAULT_PICTURES_DIR`]）、时间为 [`DEFAULT_TIME`]、配置文件不存在的平台。
    pub fn new() -> Self {
        Self::default()
    }

    /// 接入一块显示器；按添加顺序分配 `DisplayId`。
    pub fn with_display(self, display: FakeDisplay) -> Self {
        self.frames.lock().unwrap().displays.push(display);
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

    /// 模拟用户移动鼠标。
    pub fn move_cursor(&self, position: Point) {
        self.frames.lock().unwrap().cursor = position;
    }

    /// 模拟时间流逝。
    pub fn set_time(&self, time: LocalDateTime) {
        *self.time.lock().unwrap() = time;
    }

    /// 生成交给 `bps-core` 的平台能力集合，它与本对象共享状态。
    pub fn platform(&self) -> Platform {
        Platform {
            frame_source: Box::new(FakeFrameSource(self.frames.clone())),
            clipboard: Box::new(FakeClipboard(self.clipboard.clone())),
            hotkeys: Box::new(FakeHotkeys(self.hotkeys.clone())),
            file_system: Box::new(FakeFileSystem(self.files.clone())),
            notifier: Box::new(FakeNotifier(self.notifications.clone())),
            clock: Box::new(FakeClock(self.time.clone())),
            config_file: Box::new(FakeConfigFile {
                state: self.config_file.clone(),
                path: self.config_file_path(),
            }),
        }
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
}

struct FakeClock(Arc<Mutex<LocalDateTime>>);

impl Clock for FakeClock {
    fn now(&self) -> LocalDateTime {
        *self.0.lock().unwrap()
    }
}
