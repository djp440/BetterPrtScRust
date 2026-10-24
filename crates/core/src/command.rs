use std::path::PathBuf;

use serde::Deserialize;

use crate::OutputFormat;

/// 交给常驻实例执行的命令，是 `core` 的唯一入口。
///
/// 热键、CLI（经命名管道转交）、托盘菜单都先转换成命令，行为因此完全一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 按捕获模式捕获一次，然后依次执行截后动作。
    Capture(CaptureRequest),
    /// 用户再次启动了程序：激活已在运行的常驻实例。
    /// 目前常驻实例只有托盘图标、没有可显示的界面，所以什么也不做；有了设置界面后改为打开它。
    Activate,
    /// 注销热键并结束常驻实例。
    Exit,
}

/// 一次捕获要做什么：截什么（捕获模式）、截完做什么（截后动作），以及覆盖项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    /// 按顺序执行的截后动作。
    pub actions: Vec<PostCaptureAction>,
    /// 这次捕获对全局设置的覆盖；没有覆盖的项使用全局设置或内置默认值。
    pub overrides: CaptureOverrides,
}

impl CaptureRequest {
    /// 不带任何覆盖项的捕获请求。
    pub fn new(mode: CaptureMode, actions: Vec<PostCaptureAction>) -> Self {
        Self {
            mode,
            actions,
            overrides: CaptureOverrides::default(),
        }
    }
}

/// 热键绑定可以单独覆盖的设置；`None` 表示沿用全局设置或内置默认值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureOverrides {
    /// 自动存储的目录。
    pub save_dir: Option<PathBuf>,
    /// 自动存储的命名模板。
    pub filename_template: Option<String>,
    /// 产物的输出格式。
    pub output_format: Option<OutputFormat>,
    /// 录屏的帧率（每秒帧数）。
    pub frame_rate: Option<u32>,
    /// 录屏时是否显示鼠标指针。
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
    /// 所有显示器拼合成的整个虚拟桌面。尚未实现，执行时报告“暂不支持”。
    VirtualDesktop,
    /// 用户在选区遮罩上框选的区域。尚未实现，执行时报告“暂不支持”。
    Region,
    /// 用户在选区遮罩上指定的顶层窗口。尚未实现，执行时报告“暂不支持”。
    Window,
}

impl CaptureMode {
    /// 术语表里的中文名称，用于给人看的消息。
    pub fn label(self) -> &'static str {
        match self {
            CaptureMode::Display => "显示器",
            CaptureMode::VirtualDesktop => "虚拟桌面",
            CaptureMode::Region => "区域",
            CaptureMode::Window => "窗口",
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
    /// 把截图复制到剪贴板（PNG + DIB）。
    CopyToClipboard,
    /// 按命名模板和输出格式把产物写入自动存储目录。
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
    /// 术语表里的中文名称，用于给人看的消息。
    pub fn label(self) -> &'static str {
        match self {
            PostCaptureAction::CopyToClipboard => "复制到剪贴板",
            PostCaptureAction::AutoSave => "自动存储",
            PostCaptureAction::Annotate => "标注",
            PostCaptureAction::Record => "录屏",
            PostCaptureAction::LongScreenshot => "长截屏",
            PostCaptureAction::Ocr => "OCR",
            PostCaptureAction::Translate => "翻译",
        }
    }
}

/// 执行完一条命令后常驻实例应继续运行还是退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}
