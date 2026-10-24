use crate::{Image, PlatformError, Point, Rect};

/// 显示器的不透明标识，只在同一次 [`FrameSource::displays`] 结果的有效期内使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayId(pub u64);

/// 一块显示器在虚拟桌面中的位置与属性。
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub id: DisplayId,
    /// 显示器在虚拟桌面中的范围，物理像素。
    pub bounds: Rect,
    /// 系统缩放比例，例如 1.0、1.5、2.0。截图始终按物理像素输出，不受它影响。
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// 截屏能力（帧源）：提供显示器布局和鼠标位置，并截取画面。
///
/// 所有坐标和图像都是物理像素。
pub trait FrameSource {
    /// 当前接入桌面的全部显示器。
    fn displays(&self) -> Result<Vec<Display>, PlatformError>;

    /// 鼠标指针在虚拟桌面中的位置。
    fn cursor_position(&self) -> Result<Point, PlatformError>;

    /// 截取一块显示器的完整画面；图像尺寸等于该显示器 `bounds` 的物理像素尺寸。
    fn capture_display(&self, display: DisplayId) -> Result<Image, PlatformError>;
}
