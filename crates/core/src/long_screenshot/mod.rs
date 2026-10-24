//! 长截屏：用户框选区域、按“开始”后自己竖直滚动内容，程序持续采样并按相邻帧的重叠拼接。

mod stitcher;

pub use stitcher::{LongScreenshot, Stitched};
