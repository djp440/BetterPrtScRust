//! 长截屏：用户框选区域、按“开始”后自己竖直滚动内容，程序持续采样并按相邻帧的重叠拼接。

mod preview;
mod session;
mod stitcher;

pub(crate) use preview::annotation_view;
pub(crate) use session::LongScreenshotCapture;
pub use stitcher::{LongScreenshot, Stitched};
