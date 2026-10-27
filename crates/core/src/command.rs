use serde::Deserialize;

use crate::{Locale, OutputOptions, Texts};

/// 交给常驻实例执行的命令，是 `core` 的唯一入口。
///
/// 热键、CLI（经命名管道转交）、托盘菜单都先转换成命令，行为因此完全一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 按捕获模式捕获一次，再按请求的截后动作或直达功能处理。
    Capture(CaptureRequest),
    /// 用户再次启动了程序：激活已在运行的常驻实例，即打开设置界面。
    Activate,
    /// 打开设置界面（托盘菜单“设置”）；已经打开时把它带到前台。
    OpenSettings,
    /// 注销热键并结束常驻实例。
    Exit,
}

/// 一次捕获要做什么：截什么（捕获模式）、截完做什么（截后动作或直达功能）、产物怎么输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    /// 截图类捕获的截后动作，或直达功能。
    pub kind: CaptureKind,
    /// 自动保存：截图最终交付时（复制并退出、确认）同时按命名模板存进保存目录，
    /// 与截后动作无关；取消时不保存。长截屏也遵守；录屏总是保存，OCR 不保存。
    pub auto_save: bool,
    /// 输出格式、自动存储目录和命名模板。
    pub output: OutputOptions,
    /// 录屏选项；录屏（直达功能或等待确认界面上的“录屏”）用到。
    pub recording: RecordingOptions,
    /// OCR（直达热键或工具条）识别后自动复制、不弹结果面板（全局设置 `ocr_auto_copy`）。
    pub ocr_auto_copy: bool,
}

impl CaptureRequest {
    /// 使用默认输出选项（PNG、`图片\BetterPrtSc`、默认命名模板）、默认录屏选项、不自动保存的
    /// 截图类捕获请求。
    pub fn new(mode: CaptureMode, after: PostCaptureAction) -> Self {
        Self::of_kind(mode, CaptureKind::Screenshot(after))
    }

    /// 直达功能（录屏、长截屏、OCR）的捕获请求，其余同 [`CaptureRequest::new`]。
    pub fn function(mode: CaptureMode, function: DirectFunction) -> Self {
        Self::of_kind(mode, CaptureKind::Function(function))
    }

    fn of_kind(mode: CaptureMode, kind: CaptureKind) -> Self {
        Self {
            mode,
            kind,
            auto_save: false,
            output: OutputOptions::default(),
            recording: RecordingOptions::default(),
            ocr_auto_copy: false,
        }
    }

    /// 同时打开自动保存。
    pub fn with_auto_save(mut self) -> Self {
        self.auto_save = true;
        self
    }

    /// 同时打开“识别后自动复制，不弹面板”。
    pub fn with_ocr_auto_copy(mut self) -> Self {
        self.ocr_auto_copy = true;
        self
    }

    /// 是直达功能 `function`。
    pub fn is_function(&self, function: DirectFunction) -> bool {
        self.kind == CaptureKind::Function(function)
    }

    /// 是截后动作为“等待确认”的截图类捕获。
    pub fn awaits_confirmation(&self) -> bool {
        self.kind == CaptureKind::Screenshot(PostCaptureAction::AwaitConfirmation)
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
    /// MP4 是否录入系统声音（全局设置 `record_system_audio`，绑定可以覆盖）。
    pub system_audio: Option<bool>,
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
    /// 用户在选区遮罩上框选的区域：先拍冻结帧再显示遮罩，框选结果从冻结帧裁剪拼接，可以跨显示器。
    Region,
    /// 用户在选区遮罩上单击选中的顶层窗口：截取它的完整内容（被遮挡的部分也在内），
    /// 不含阴影，圆角以外透明；命名模板的 `{window_title}` 是它的标题。
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

/// 一条绑定做什么：截图类（选一种截后动作）或直达功能。
///
/// 配置文件里截图类写 `after = "..."`，直达功能写 `function = "..."`，两者互斥。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureKind {
    /// 截图，完成后执行截后动作。
    Screenshot(PostCaptureAction),
    /// 热键直接进入的功能。
    Function(DirectFunction),
}

impl CaptureKind {
    /// 全部种类，按设置界面的顺序排列。
    pub const ALL: [CaptureKind; 5] = [
        CaptureKind::Screenshot(PostCaptureAction::CopyToClipboard),
        CaptureKind::Screenshot(PostCaptureAction::AwaitConfirmation),
        CaptureKind::Function(DirectFunction::Record),
        CaptureKind::Function(DirectFunction::LongScreenshot),
        CaptureKind::Function(DirectFunction::Ocr),
    ];

    /// 界面上显示的名称。
    pub fn localized_label(self, texts: &Texts) -> String {
        match self {
            CaptureKind::Screenshot(after) => after.localized_label(texts),
            CaptureKind::Function(function) => function.localized_label(texts),
        }
    }

    /// 自动保存对这种捕获有意义：截图类和长截屏（录屏总是保存，OCR 不保存）。
    pub fn supports_auto_save(self) -> bool {
        !matches!(
            self,
            CaptureKind::Function(DirectFunction::Record | DirectFunction::Ocr)
        )
    }
}

/// 截后动作：截图完成（框选松开、单击窗口、显示器截取）后做什么。每条截图类绑定选其一。
///
/// 配置文件里写作 `copy_to_clipboard`、`await_confirmation`。自动保存是独立的开关
/// （[`CaptureRequest::auto_save`]），与截后动作无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostCaptureAction {
    /// 直接把截图复制到剪贴板（按输出格式编码的图像 + DIB）并退出截图界面。
    CopyToClipboard,
    /// 停留在截图界面，选区上显示工具条：标注、录屏、长截屏、OCR、翻译、另存为、确认、取消。
    /// 确认（Enter / 工具条“确认”）把截图复制到剪贴板并退出；取消（Esc / 工具条“取消”）不复制。
    AwaitConfirmation,
}

impl PostCaptureAction {
    /// 全部截后动作。
    pub const ALL: [PostCaptureAction; 2] = [
        PostCaptureAction::CopyToClipboard,
        PostCaptureAction::AwaitConfirmation,
    ];

    /// 术语表里的中文名称，用于 CLI 和日志。
    pub fn label(self) -> String {
        self.localized_label(Texts::of(Locale::ZhCn))
    }

    /// 界面上显示的名称。
    pub fn localized_label(self, texts: &Texts) -> String {
        texts.get(match self {
            PostCaptureAction::CopyToClipboard => "action-copy-to-clipboard",
            PostCaptureAction::AwaitConfirmation => "action-await-confirmation",
        })
    }

    /// 配置文件里的写法。
    pub fn config_name(self) -> &'static str {
        match self {
            PostCaptureAction::CopyToClipboard => "copy_to_clipboard",
            PostCaptureAction::AwaitConfirmation => "await_confirmation",
        }
    }

    /// 解析配置文件里的写法。
    pub fn from_config_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.config_name() == name)
    }
}

/// 直达功能：热键不经过截后动作，直接进入的功能。
///
/// 配置文件里写作 `record`、`long_screenshot`、`ocr`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectFunction {
    /// 在实时画面上框选后开始录屏。
    Record,
    /// 用户滚动内容时连续采样并拼接成长截屏。
    LongScreenshot,
    /// 识别截图中的文字（打开结果面板）。
    Ocr,
}

impl DirectFunction {
    /// 全部直达功能。
    pub const ALL: [DirectFunction; 3] = [
        DirectFunction::Record,
        DirectFunction::LongScreenshot,
        DirectFunction::Ocr,
    ];

    /// 术语表里的中文名称，用于 CLI 和日志。
    pub fn label(self) -> String {
        self.localized_label(Texts::of(Locale::ZhCn))
    }

    /// 界面上显示的名称。
    pub fn localized_label(self, texts: &Texts) -> String {
        texts.get(match self {
            DirectFunction::Record => "action-record",
            DirectFunction::LongScreenshot => "action-long-screenshot",
            DirectFunction::Ocr => "action-ocr",
        })
    }

    /// 配置文件里的写法。
    pub fn config_name(self) -> &'static str {
        match self {
            DirectFunction::Record => "record",
            DirectFunction::LongScreenshot => "long_screenshot",
            DirectFunction::Ocr => "ocr",
        }
    }

    /// 解析配置文件里的写法。
    pub fn from_config_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.config_name() == name)
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
