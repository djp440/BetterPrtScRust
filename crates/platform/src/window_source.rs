//! 窗口枚举与命中测试能力：桌面上有哪些顶层窗口、光标下是哪个窗口，以及截取一个窗口的完整内容。
//!
//! 坐标一律是虚拟桌面物理像素，与 [`crate::FrameSource`] 一致。

use crate::{Image, PlatformError, Point, Rect};

/// 顶层窗口的不透明标识（Windows 上是 `HWND`）。窗口随时可能被关闭，用它截图或查询时要处理失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

/// 一个顶层窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopLevelWindow {
    pub id: WindowId,
    /// 窗口标题，可以为空。
    pub title: String,
    /// 窗口在屏幕上可见部分的外框（不含阴影和不可见的缩放边框），虚拟桌面物理像素；
    /// 可以超出虚拟桌面。窗口截图的尺寸与它一致。
    pub bounds: Rect,
}

/// 某一时刻桌面上用户看得见的顶层窗口，按 Z 序从上到下排列。
///
/// 不含隐藏、最小化、被系统隐藏（cloaked）、鼠标可以穿透的窗口，也不含本程序自己的窗口
/// （选区遮罩等）。窗口之间的遮挡关系就是排列顺序：靠前的盖在靠后的上面。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowLayout {
    windows: Vec<TopLevelWindow>,
}

impl WindowLayout {
    /// `windows` 按 Z 序从上到下排列。
    pub fn new(windows: Vec<TopLevelWindow>) -> Self {
        Self { windows }
    }

    /// 全部窗口，按 Z 序从上到下。
    pub fn windows(&self) -> &[TopLevelWindow] {
        &self.windows
    }

    /// 命中测试：`point` 处用户看到的顶层窗口，即外框包含 `point` 的最上层窗口；
    /// 那里只有桌面时返回 `None`。只命中顶层窗口，不深入到窗口里的控件。
    pub fn window_at(&self, point: Point) -> Option<&TopLevelWindow> {
        self.windows.iter().find(|w| w.bounds.contains(point))
    }

    /// 按标识查找窗口。
    pub fn get(&self, id: WindowId) -> Option<&TopLevelWindow> {
        self.windows.iter().find(|w| w.id == id)
    }
}

/// 窗口枚举与命中测试能力。
pub trait WindowSource {
    /// 此刻桌面上的顶层窗口布局（枚举）。命中测试用返回值的 [`WindowLayout::window_at`]。
    ///
    /// 在冻结帧上选择窗口时，应在拍冻结帧的同时取一次布局，之后都用这一份，
    /// 使高亮与用户看到的冻结帧一致。
    fn layout(&self) -> Result<WindowLayout, PlatformError>;

    /// 窗口里可见的控件（子窗口，含嵌套的）在屏幕上的外框，虚拟桌面物理像素，顺序不保证；
    /// 没有控件时为空。供框选区域时吸附到控件边缘使用。窗口已关闭时报错。
    fn controls(&self, window: WindowId) -> Result<Vec<Rect>, PlatformError>;

    /// 截取窗口此刻的完整内容：被其他窗口遮挡、超出屏幕的部分也在内，不含窗口阴影；
    /// 圆角以外的像素是透明的。图像尺寸等于截图那一刻窗口外框（[`TopLevelWindow::bounds`]）的尺寸。
    fn capture_window(&self, window: WindowId) -> Result<Image, PlatformError>;
}
