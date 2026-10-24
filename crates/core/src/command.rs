use serde::Deserialize;

use crate::{Locale, OutputOptions, Texts};

/// 交给常驻实例执行的命令，是 `core` 的唯一入口。
///
/// 热键、CLI（经命名管道转交）、托盘菜单都先转换成命令，行为因此完全一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 按捕获模式捕获一次，然后依次执行截后动作。
    Capture(CaptureRequest),
    /// 用户再次启动了程序：激活已在运行的常驻实例，即打开设置界面。
    Activate,
    /// 打开设置界面（托盘菜单“设置”）；已经打开时把它带到前台。
    OpenSettings,
    /// 注销热键并结束常驻实例。
    Exit,
}

/// 一次捕获要做什么：截什么（捕获模式）、截完做什么（截后动作）、产物怎么输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    /// 截后动作。复制到剪贴板与自动存储一起处理：剪贴板只写一次，
    /// 同时做了自动存储时附带存下的文件。
    pub actions: Vec<PostCaptureAction>,
    /// 输出格式、自动存储目录和命名模板。
    pub output: OutputOptions,
    /// 录屏选项；只有带录屏动作的捕获用到。
    pub recording: RecordingOptions,
}

impl CaptureRequest {
    /// 使用默认输出选项（PNG、`图片\BetterPrtSc`、默认命名模板）和默认录屏选项的捕获请求。
    pub fn new(mode: CaptureMode, actions: Vec<PostCaptureAction>) -> Self {
        Self {
            mode,
            actions,
            output: OutputOptions::default(),
            recording: RecordingOptions::default(),
        }
    }
}

/// 录屏选项；`None` 表示使用录屏功能的内置默认值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingOptions {
    /// 录屏的输出格式。
    pub format: Option<RecordingFormat>,
    /// 帧率（每秒帧数）。
    pub frame_rate: Option<u32>,
    /// 是否录入鼠标指针。
    pub show_cursor: Option<bool>,
}

/// 捕获模式：决定捕获屏幕上哪一部分内容。
///
/// 配置文件里写作 `display`、`virtual_desktop`、`region`、`window`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// 鼠标所在的那块显示器，按物理像素输出。
    Display,
    /// 所有显示器拼合成的整个虚拟桌面：各显示器按物理像素截取后按位置拼接，
    /// 显示器之间没有覆盖的空隙是透明像素。
    VirtualDesktop,
    /// 用户在选区遮罩上框选的区域。尚未实现，执行时报告“暂不支持”。
    Region,
    /// 用户在选区遮罩上指定的顶层窗口。尚未实现，执行时报告“暂不支持”。
    Window,
}

impl CaptureMode {
    /// 全部捕获模式，按术语表的顺序排列。
    pub const ALL: [CaptureMode; 4] = [
        CaptureMode::Display,
        CaptureMode::VirtualDesktop,
        CaptureMode::Region,
        CaptureMode::Window,
    ];

    /// 术语表里的中文名称，用于 CLI 和日志。
    pub fn label(self) -> String {
        self.localized_label(Texts::of(Locale::ZhCn))
    }

    /// 界面上显示的名称。
    pub fn localized_label(self, texts: &Texts) -> String {
        texts.get(match self {
            CaptureMode::Display => "mode-display",
            CaptureMode::VirtualDesktop => "mode-virtual-desktop",
            CaptureMode::Region => "mode-region",
            CaptureMode::Window => "mode-window",
        })
    }

    /// 配置文件里的写法。
    pub fn config_name(self) -> &'static str {
        match self {
            CaptureMode::Display => "display",
            CaptureMode::VirtualDesktop => "virtual_desktop",
            CaptureMode::Region => "region",
            CaptureMode::Window => "window",
        }
    }
}

/// 截后动作：捕获完成后对产物执行的处理。
///
/// 配置文件里写作 `copy_to_clipboard`、`auto_save`、`annotate`、`record`、
/// `long_screenshot`、`ocr`、`translate`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostCaptureAction {
    /// 把截图复制到剪贴板：按输出格式编码的图像 + DIB。
    CopyToClipboard,
    /// 自动存储：按命名模板和输出格式写入存储目录，完成后发通知。
    AutoSave,
    /// 进入标注。
    Annotate,
    /// 在实时画面上框选后开始录屏。
    Record,
    /// 用户滚动内容时连续采样并拼接成长截屏。
    LongScreenshot,
    /// 识别截图中的文字。
    Ocr,
    /// 把 OCR 结果交给翻译服务。
    Translate,
}

impl PostCaptureAction {
    /// 全部截后动作，按术语表的顺序排列。
    pub const ALL: [PostCaptureAction; 7] = [
        PostCaptureAction::CopyToClipboard,
        PostCaptureAction::AutoSave,
        PostCaptureAction::Annotate,
        PostCaptureAction::Record,
        PostCaptureAction::LongScreenshot,
        PostCaptureAction::Ocr,
        PostCaptureAction::Translate,
    ];

    /// 术语表里的中文名称，用于 CLI 和日志。
    pub fn label(self) -> String {
        self.localized_label(Texts::of(Locale::ZhCn))
    }

    /// 界面上显示的名称。
    pub fn localized_label(self, texts: &Texts) -> String {
        texts.get(match self {
            PostCaptureAction::CopyToClipboard => "action-copy-to-clipboard",
            PostCaptureAction::AutoSave => "action-auto-save",
            PostCaptureAction::Annotate => "action-annotate",
            PostCaptureAction::Record => "action-record",
            PostCaptureAction::LongScreenshot => "action-long-screenshot",
            PostCaptureAction::Ocr => "action-ocr",
            PostCaptureAction::Translate => "action-translate",
        })
    }

    /// 配置文件里的写法。
    pub fn config_name(self) -> &'static str {
        match self {
            PostCaptureAction::CopyToClipboard => "copy_to_clipboard",
            PostCaptureAction::AutoSave => "auto_save",
            PostCaptureAction::Annotate => "annotate",
            PostCaptureAction::Record => "record",
            PostCaptureAction::LongScreenshot => "long_screenshot",
            PostCaptureAction::Ocr => "ocr",
            PostCaptureAction::Translate => "translate",
        }
    }
}

/// 录屏的输出格式。截图的输出格式见 [`crate::OutputFormat`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingFormat {
    Mp4,
    Gif,
}

/// 执行完一条命令后常驻实例应继续运行还是退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}
