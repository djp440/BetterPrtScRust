/// 交给常驻实例执行的命令，是 `core` 的唯一入口。
///
/// 热键、CLI（经命名管道转交）、托盘菜单都先转换成命令，行为因此完全一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 按捕获模式捕获一次，然后依次执行截后动作。
    Capture(CaptureRequest),
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
