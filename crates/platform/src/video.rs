//! 录屏用的平台能力：持续捕获画面的视频源、把帧编码成视频文件的视频编码器，
//! 以及驱动录屏按帧率取帧的节拍。
//!
//! 分工：`core` 决定什么时候取帧、写哪一帧、时间戳是多少（恒定帧率、停止、以后的暂停）；
//! 平台只负责“给出当前最新的画面”和“把给定的帧按时间戳编码进文件”。帧（[`VideoFrame`]）
//! 对 `core` 是不透明的：Windows 上它是 GPU 纹理，从视频源到编码器不经过 CPU。

use std::any::Any;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use crate::{AudioFormat, Image, PlatformError, Rect, WindowId};

/// 录屏要持续捕获的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoTarget {
    /// 虚拟桌面上的一块矩形（物理像素），可以跨越多块显示器；帧的尺寸等于矩形的尺寸。
    Region(Rect),
    /// 一个顶层窗口自己的内容（被其他窗口遮挡、超出屏幕的部分也在内，不含阴影和鼠标指针以外的
    /// 屏幕内容）。帧的尺寸固定为 `width` × `height`（开始录制时窗口的尺寸，由调用方决定）：
    /// 窗口之后改变大小时不缩放，窗口内容贴在画布左上角，窗口变大时超出的部分被裁掉，
    /// 变小时画布其余部分是黑色。窗口关闭后流返回 [`StreamFrame::Ended`]。
    Window {
        window: WindowId,
        width: u32,
        height: u32,
    },
}

/// 打开视频流时的选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    /// 画面里是否包含鼠标指针。
    pub show_cursor: bool,
}

/// 视频源：持续捕获屏幕上的一部分（录屏用）。截图用的是 [`crate::FrameSource`]。
pub trait VideoSource {
    /// 开始捕获 `target`。返回的流被丢弃时停止捕获、释放资源。
    fn open(
        &mut self,
        target: VideoTarget,
        options: StreamOptions,
    ) -> Result<Box<dyn VideoStream>, PlatformError>;
}

/// 一路正在捕获的画面。
pub trait VideoStream {
    /// 此刻最新的画面。画面没有变化时返回与上一次相同的帧（同一个 `Rc`），不重复拷贝。
    fn latest(&mut self) -> Result<StreamFrame, PlatformError>;
}

/// [`VideoStream::latest`] 的结果。
#[derive(Clone)]
pub enum StreamFrame {
    /// 最新的一帧，尺寸固定为打开时目标的尺寸。
    Frame(Rc<dyn VideoFrame>),
    /// 刚开始捕获，还没有收到第一帧。
    Pending,
    /// 捕获的目标已经不存在（例如显示器被拔掉、显示器布局变了），不会再有新的帧。
    Ended,
}

/// 视频源给出的一帧画面。对 `core` 不透明，只能原样交给同一平台的 [`VideoWriter`]。
pub trait VideoFrame {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// 读回成 RGBA 图像。比较慢（Windows 上要从显存读回），只在需要 CPU 像素时使用。
    fn to_image(&self) -> Result<Image, PlatformError>;
    /// 平台内部的表示（例如 GPU 纹理），供同一平台的编码器取用。
    fn as_any(&self) -> &dyn Any;
}

/// 视频文件的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoFormat {
    /// MP4 文件（视频编码见 [`VideoCodec`]），可以带一条 AAC 音轨。
    Mp4,
    /// GIF 动图，纯 CPU 编码；每帧最多 256 色，没有声音。
    Gif,
}

/// MP4 的视频编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoCodec {
    /// H.264，Windows 都自带编码器。
    #[default]
    H264,
    /// H.265（HEVC），需要显卡的硬件编码器或系统的 HEVC 扩展。
    H265,
    /// AV1，需要支持 AV1 编码的显卡。
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1];

    /// 给用户看的名字。
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::H265 => "H.265",
            VideoCodec::Av1 => "AV1",
        }
    }
}

/// 一段视频的编码参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSettings {
    pub format: VideoFormat,
    /// 画面尺寸（像素）；写入的每一帧都必须是这个尺寸。
    pub width: u32,
    pub height: u32,
    /// 恒定帧率（每秒帧数）。
    pub frame_rate: u32,
    /// 视频编码（只对 [`VideoFormat::Mp4`] 有意义）。
    pub codec: VideoCodec,
    /// 目标码率（比特每秒，只对 [`VideoFormat::Mp4`] 有意义）。
    pub bitrate: u32,
    /// 音轨的格式；`None` 表示没有音轨。只有 [`VideoFormat::Mp4`] 能带音轨。
    pub audio: Option<AudioFormat>,
}

/// 视频编码器。
pub trait VideoEncoder {
    /// 开始把视频编码进 `path`：文件已经存在时覆盖它（调用方先建好一个空文件占住文件名）。
    fn start(
        &mut self,
        path: &Path,
        settings: &VideoSettings,
    ) -> Result<Box<dyn VideoWriter>, PlatformError>;
}

/// 一段正在编码的视频。没有 [`finish`](Self::finish) 就被丢弃时放弃编码，文件内容不可用。
pub trait VideoWriter {
    /// 写入一帧。`timestamp` 是这一帧从视频开头算起的显示时间，逐帧严格递增；
    /// 帧一直显示到下一帧的时间戳（最后一帧显示 1 / 帧率 秒）。
    fn write(&mut self, frame: &dyn VideoFrame, timestamp: Duration) -> Result<(), PlatformError>;

    /// 往音轨里追加样本（[`VideoSettings::audio`] 格式、交错排列）。音轨从视频开头开始，
    /// 样本首尾相接、没有时间戳：空白由调用方补上静音。没有音轨时返回错误。
    fn write_audio(&mut self, samples: &[f32]) -> Result<(), PlatformError>;

    /// 写完全部数据并关闭文件，之后文件就是可以播放的完整视频。
    fn finish(self: Box<Self>) -> Result<(), PlatformError>;
}

/// 节拍：开始后每隔一个周期由平台事件循环递交一次
/// [`PlatformEvent::Tick`](crate::PlatformEvent::Tick)，录屏按它取帧、写帧。
///
/// 同一时间只有一个节拍；节拍可能晚到，也可能因为处理不及时而合并，所以收到节拍的一方
/// 应当按 [`crate::Clock::monotonic`] 判断该做什么，而不是数节拍。
pub trait Ticker {
    /// 开始（或改为）每隔 `period` 递交一次节拍。
    fn start(&mut self, period: Duration) -> Result<(), PlatformError>;
    /// 停止递交节拍；已经排进队列的节拍仍可能递交。没有开始时什么也不做。
    fn stop(&mut self);
}
