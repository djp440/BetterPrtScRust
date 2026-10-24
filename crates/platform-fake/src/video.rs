//! 假视频源、假视频编码器与假节拍。
//!
//! - 假视频源的每一帧都是调用 `latest` 那一刻假显示器画面上目标区域的副本；需要时把鼠标指针
//!   画成指针所在位置的一个 [`FAKE_CURSOR_COLOR`] 像素（见 [`draw_fake_cursor`]）。
//! - 假编码器把收到的帧和时间戳原样记下，`finish` 时按 [`crate::decode::video`] 能读回的
//!   简单格式写进假文件系统；没有 `finish` 就被丢弃时什么也不写。
//! - 假节拍只记录周期，节拍事件由测试用 [`crate::FakePlatform::tick`] 生成。

use std::any::Any;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bps_platform::{
    Image, PlatformError, Point, Rect, StreamFrame, StreamOptions, Ticker, VideoEncoder,
    VideoFormat, VideoFrame, VideoSettings, VideoSource, VideoStream, VideoTarget, VideoWriter,
    WindowId,
};

use crate::file_system::FsState;
use crate::frame_source::FrameState;
use crate::window_source::WindowSourceState;

/// 假视频源画鼠标指针用的颜色：指针所在的那一个像素。
pub const FAKE_CURSOR_COLOR: [u8; 4] = [255, 0, 255, 255];

/// 假视频文件开头的标记，后面跟格式名。
pub(crate) const FAKE_VIDEO_MAGIC: &[u8; 8] = b"FAKEVID\0";

/// 在 `image`（左上角位于虚拟桌面 `origin`）上按假视频源的方式画出位于 `cursor` 的鼠标指针；
/// 指针不在图像范围内时原样返回。用来计算录屏帧的期望值。
pub fn draw_fake_cursor(image: &Image, origin: Point, cursor: Point) -> Image {
    let (x, y) = (cursor.x - origin.x, cursor.y - origin.y);
    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
        return image.clone();
    }
    let mut rgba = image.rgba().to_vec();
    let i = (y as usize * image.width() as usize + x as usize) * 4;
    rgba[i..i + 4].copy_from_slice(&FAKE_CURSOR_COLOR);
    Image::from_rgba(image.width(), image.height(), rgba)
}

/// 视频相关的假平台状态。
#[derive(Default)]
pub(crate) struct VideoState {
    /// 还没有关闭的视频流数量。
    pub open_streams: usize,
    /// 视频源还没收到第一帧（`latest` 返回 `Pending`）。
    pub pending: bool,
    /// 捕获的目标已经消失（`latest` 返回 `Ended`）。
    pub ended: bool,
    /// 开始编码时失败的原因。
    pub encoder_error: Option<String>,
    /// 节拍的周期；`None` 表示没有开始。
    pub ticker: Option<Duration>,
}

pub(crate) struct FakeVideoSource {
    pub windows: Arc<Mutex<WindowSourceState>>,
    pub frames: Arc<Mutex<FrameState>>,
    pub state: Arc<Mutex<VideoState>>,
}

impl VideoSource for FakeVideoSource {
    fn open(
        &mut self,
        target: VideoTarget,
        options: StreamOptions,
    ) -> Result<Box<dyn VideoStream>, PlatformError> {
        let (source, width, height) = match target {
            VideoTarget::Region(rect) => (Source::Region(rect), rect.width, rect.height),
            VideoTarget::Window {
                window,
                width,
                height,
            } => {
                if self.windows.lock().unwrap().image_of(window).is_none() {
                    return Err(PlatformError::new("假视频源：窗口已关闭"));
                }
                (Source::Window(window), width, height)
            }
        };
        if width == 0 || height == 0 {
            return Err(PlatformError::new("假视频源：区域为空"));
        }
        self.state.lock().unwrap().open_streams += 1;
        Ok(Box::new(FakeVideoStream {
            frames: self.frames.clone(),
            windows: self.windows.clone(),
            state: self.state.clone(),
            source,
            width,
            height,
            show_cursor: options.show_cursor,
        }))
    }
}

enum Source {
    Region(Rect),
    Window(WindowId),
}

struct FakeVideoStream {
    frames: Arc<Mutex<FrameState>>,
    windows: Arc<Mutex<WindowSourceState>>,
    state: Arc<Mutex<VideoState>>,
    source: Source,
    width: u32,
    height: u32,
    show_cursor: bool,
}

impl VideoStream for FakeVideoStream {
    fn latest(&mut self) -> Result<StreamFrame, PlatformError> {
        {
            let state = self.state.lock().unwrap();
            if state.ended {
                return Ok(StreamFrame::Ended);
            }
            if state.pending {
                return Ok(StreamFrame::Pending);
            }
        }
        let image = match self.source {
            Source::Region(rect) => {
                let frames = self.frames.lock().unwrap();
                let mut image = frames.crop(rect);
                if self.show_cursor {
                    image = draw_fake_cursor(&image, Point::new(rect.x, rect.y), frames.cursor);
                }
                image
            }
            Source::Window(window) => {
                let content = self.windows.lock().unwrap().image_of(window);
                match content {
                    Some(content) => fit_to_canvas(&content, self.width, self.height),
                    None => return Ok(StreamFrame::Ended),
                }
            }
        };
        Ok(StreamFrame::Frame(Rc::new(FakeFrame(image))))
    }
}

/// 窗口内容贴在 `width` × `height` 画布的左上角：超出的部分裁掉，不足的部分是不透明黑色。
fn fit_to_canvas(content: &Image, width: u32, height: u32) -> Image {
    let mut rgba = [0, 0, 0, 255].repeat(width as usize * height as usize);
    let columns = content.width().min(width) as usize * 4;
    for y in 0..content.height().min(height) as usize {
        let from = y * content.width() as usize * 4;
        let to = y * width as usize * 4;
        rgba[to..to + columns].copy_from_slice(&content.rgba()[from..from + columns]);
    }
    Image::from_rgba(width, height, rgba)
}

impl Drop for FakeVideoStream {
    fn drop(&mut self) {
        self.state.lock().unwrap().open_streams -= 1;
    }
}

/// 假视频源的一帧：就是一张图像。
struct FakeFrame(Image);

impl VideoFrame for FakeFrame {
    fn width(&self) -> u32 {
        self.0.width()
    }

    fn height(&self) -> u32 {
        self.0.height()
    }

    fn to_image(&self) -> Result<Image, PlatformError> {
        Ok(self.0.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) struct FakeVideoEncoder {
    pub files: Arc<Mutex<FsState>>,
    pub state: Arc<Mutex<VideoState>>,
}

impl VideoEncoder for FakeVideoEncoder {
    fn start(
        &mut self,
        path: &Path,
        settings: &VideoSettings,
    ) -> Result<Box<dyn VideoWriter>, PlatformError> {
        if let Some(message) = &self.state.lock().unwrap().encoder_error {
            return Err(PlatformError::new(message.clone()));
        }
        if settings.width == 0 || settings.height == 0 || settings.frame_rate == 0 {
            return Err(PlatformError::new(format!(
                "假编码器：无效的参数 {settings:?}"
            )));
        }
        Ok(Box::new(FakeVideoWriter {
            files: self.files.clone(),
            path: path.to_path_buf(),
            settings: *settings,
            frames: Vec::new(),
        }))
    }
}

struct FakeVideoWriter {
    files: Arc<Mutex<FsState>>,
    path: PathBuf,
    settings: VideoSettings,
    frames: Vec<(Duration, Image)>,
}

impl VideoWriter for FakeVideoWriter {
    fn write(&mut self, frame: &dyn VideoFrame, timestamp: Duration) -> Result<(), PlatformError> {
        if (frame.width(), frame.height()) != (self.settings.width, self.settings.height) {
            return Err(PlatformError::new(format!(
                "假编码器：帧尺寸 {}x{} 与视频尺寸 {}x{} 不符",
                frame.width(),
                frame.height(),
                self.settings.width,
                self.settings.height
            )));
        }
        if self
            .frames
            .last()
            .is_some_and(|(last, _)| timestamp <= *last)
        {
            return Err(PlatformError::new(format!(
                "假编码器：时间戳 {timestamp:?} 没有严格递增"
            )));
        }
        self.frames.push((timestamp, frame.to_image()?));
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(), PlatformError> {
        let mut bytes = FAKE_VIDEO_MAGIC.to_vec();
        bytes.extend_from_slice(match self.settings.format {
            VideoFormat::Mp4 => b"mp4\0",
            VideoFormat::Gif => b"gif\0",
        });
        for value in [
            self.settings.width,
            self.settings.height,
            self.settings.frame_rate,
            self.frames.len() as u32,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for (timestamp, image) in &self.frames {
            bytes.extend_from_slice(&(timestamp.as_nanos() as u64).to_le_bytes());
            bytes.extend_from_slice(image.rgba());
        }
        self.files.lock().unwrap().overwrite(&self.path, bytes)
    }
}

pub(crate) struct FakeTicker(pub Arc<Mutex<VideoState>>);

impl Ticker for FakeTicker {
    fn start(&mut self, period: Duration) -> Result<(), PlatformError> {
        if period.is_zero() {
            return Err(PlatformError::new("假节拍：周期必须大于 0"));
        }
        self.0.lock().unwrap().ticker = Some(period);
        Ok(())
    }

    fn stop(&mut self) {
        self.0.lock().unwrap().ticker = None;
    }
}
