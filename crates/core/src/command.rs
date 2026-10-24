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

/// 一次捕获要做什么：截什么（捕获模式）、截完做什么（截后动作）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    /// 按顺序执行的截后动作。
    pub actions: Vec<PostCaptureAction>,
}

/// 捕获模式：决定捕获屏幕上哪一部分内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostCaptureAction {
    /// 把截图复制到剪贴板（PNG + DIB）。
    CopyToClipboard,
}

/// 执行完一条命令后常驻实例应继续运行还是退出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}
