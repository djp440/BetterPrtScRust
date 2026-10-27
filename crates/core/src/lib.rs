//! BetterPrtSc 平台无关的核心逻辑。
//!
//! 唯一入口是 [`Command`]：热键、CLI、托盘菜单等所有触发方式都先转换成命令，
//! 再交给 [`ResidentInstance::execute`] 执行。平台能力通过 `bps_platform::Platform`
//! 注入，测试时换成 `bps-platform-fake`。
//!
//! 热键绑定来自配置文件 [`Config`]：[`ResidentInstance::start`] 读取（首次运行时写入
//! [`DEFAULT_CONFIG`]）并注册热键，配置文件被外部修改后自动重新加载。
//!
//! 其他进程（`bps` CLI、再次启动的程序）用 [`forward`] / [`run_cli`] 把 [`Request`]
//! 经 `bps_platform::ResidentChannel` 转交给常驻实例；常驻实例收到的请求事件由
//! [`ResidentInstance::handle`] 转换成命令执行，并回复 [`Reply`]。
//!
//! 开机自启与以管理员身份运行由配置文件的 `autostart` / `run_as_admin` 决定，常驻实例启动时和
//! 配置变化后按它们登记或移除开机自启（`bps_platform::Autostart`）；卸载前用
//! [`remove_autostart`] 清理。PrtSc 被系统截图工具占用时常驻实例发出引导通知，用户在系统设置里
//! 关闭后自动接管 PrtSc。
//!
//! 跨显示器拼接以公共函数提供，供虚拟桌面与区域捕获共用：[`capture_rect`] 按虚拟桌面坐标矩形
//! 截图，[`stitch`] 把已截下的各显示器画面（[`DesktopImage`]）按位置拼接并裁剪。
//!
//! 区域捕获要等用户在选区遮罩（`bps_platform::SelectionOverlay`）上框选：命令打开遮罩后即返回，
//! 之后平台递交的 `PlatformEvent::OverlayInput` 推进框选状态机，确定后从冻结帧裁剪拼接。
//!
//! 截后动作含“录屏”的捕获开始一次录屏（见 `recording` 模块）：区域模式在实时画面上的遮罩
//! （`Backdrop::Live`）里框选，然后从 `bps_platform::VideoSource` 取画面、由
//! `bps_platform::VideoEncoder` 编码，`bps_platform::Ticker` 按帧率递交的 `PlatformEvent::Tick`
//! 推进时间轴；再次触发同一个捕获请求时停止，按命名模板存储并把文件放进剪贴板。
//!
//! 截后动作包含标注时，框好区域后旁边浮现标注工具条，用户在遮罩上画标注，最后选择复制到
//! 剪贴板、自动存储或另存为结束捕获。标注文档模型 [`AnnotationDocument`] 把标注列表合成为
//! 最终截图；遮罩上显示的也是它合成的像素（遮罩的画布），屏幕上看到的就是最终产物。
//!
//! 窗口捕获同样在遮罩上进行：悬停高亮光标下的顶层窗口（`bps_platform::WindowSource` 给出的
//! 窗口布局），单击后关闭遮罩，再截取这个窗口的完整内容。

mod action;
mod active;
mod annotation;
mod autostart;
mod binding;
mod capture;
mod cli;
mod command;
mod config;
mod downscale;
mod encode;
mod error;
mod filename;
mod forward;
mod hotkey_recorder;
mod i18n;
mod instance;
mod location;
mod long_screenshot;
mod magnifier;
mod output;
mod recording;
mod region;
mod request;
mod result_panel;
mod selection;
mod settings;
mod snap;
mod status;
mod stitch;
mod translation;
mod update;

pub use annotation::{Annotation, AnnotationDocument, Stroke};
pub use autostart::remove_autostart;
pub use binding::{HotkeyBinding, HotkeyRegistrationFailure, RegistrationError, default_bindings};
pub use cli::{CliOutcome, CliStatus, cli_locale, run_cli};
pub use command::{
    CaptureKind, CaptureMode, CaptureRequest, Command, DirectFunction, Flow, PostCaptureAction,
    RecordingFormat, RecordingOptions, VideoQuality,
};
pub use config::{Config, ConfigError, DEFAULT_CONFIG};
pub use error::CoreError;
pub use filename::{
    DEFAULT_FILENAME_TEMPLATE, EXAMPLE_CAPTURE_TIME, FilenameFields, FilenameTemplate,
    FilenameTemplateError,
};
pub use forward::{ForwardError, WhenNotRunning, forward};
pub use hotkey_recorder::{HotkeyRecorder, RecordingOutcome};
pub use i18n::{Language, Locale, Texts, UnknownLanguage};
pub use instance::{ResidentInstance, StartOptions};
pub use location::{DataLocation, PORTABLE_MARKER};
pub use long_screenshot::{LongScreenshot, Stitched};
pub use output::{OutputFormat, OutputOptions, SaveDir};
pub use request::{HOTKEY_PAUSE_LEASE, Reply, Request};
pub use result_panel::{
    OcrCopy, Recognition, RecognitionOutcome, RecognitionStatus, ResultPanel, Translation,
    TranslationOutcome, TranslationStatus, copy_recognized_text,
};
pub use settings::{
    BindingSettings, RecordingSettings, Settings, SettingsDocument, SettingsProblem,
    set_ocr_auto_copy,
};
pub use status::{BindingFailure, ResidentStatus, UpdateStatus};
pub use stitch::{DesktopImage, capture_rect, stitch, virtual_desktop_bounds};
pub use translation::{TargetLanguage, TranslationConfig, TranslationError, TranslationProvider};
pub use update::VERSION;
