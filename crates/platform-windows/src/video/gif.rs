//! GIF 编码器：纯 Rust（`gif` crate），在 CPU 上编码，不依赖 Media Foundation，也不捆绑 ffmpeg。
//!
//! 每一帧先读回成 RGBA，再与上一帧比较：完全相同的帧不写出（只延长上一帧的停留时间），
//! 有变化的帧只写出变化的外接矩形（帧间优化，录桌面时文件小得多），矩形内用 NeuQuant
//! 量化成 256 色局部调色板。GIF 的帧延迟以 1/100 秒为单位，这里按累计时间戳取整，避免误差累积。

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::Duration;

use bps_platform::{Image, PlatformError, VideoFrame, VideoSettings, VideoWriter};
use gif::{DisposalMethod, Encoder, Frame, Repeat};

/// NeuQuant 的采样间隔（1–30，越大越快、质量越低）。录屏要实时跟上节拍，取 10。
const QUANTIZE_SPEED: i32 = 10;
/// 帧延迟的下限（1/100 秒）：浏览器和看图软件通常把小于 2 的延迟当成 10，所以不写更小的值。
const MIN_DELAY: u64 = 2;
/// GIF 帧延迟字段（u16）能表示的上限（1/100 秒）。
const MAX_DELAY: u64 = u16::MAX as u64;

/// 已经量化好、还没写出的一帧：写出时才知道它要停留多久。
struct Pending {
    /// 这一帧开始显示的时间（视频时间轴）。
    start: Duration,
    frame: Frame<'static>,
}

pub(crate) struct GifWriter {
    encoder: Encoder<BufWriter<File>>,
    width: u32,
    height: u32,
    frame_period: Duration,
    /// 画布此刻的内容（最近一次有变化的帧）。
    canvas: Option<Image>,
    pending: Option<Pending>,
    /// 最近一次写入的帧的时间戳（包括没有变化而被合并的帧）。
    last_timestamp: Duration,
}

impl GifWriter {
    pub(crate) fn start(path: &Path, settings: &VideoSettings) -> Result<Self, PlatformError> {
        let size = |value: u32, what: &str| {
            u16::try_from(value).map_err(|_| {
                PlatformError::new(format!("GIF 的{what}不能超过 65535 像素（当前 {value}）"))
            })
        };
        let (width, height) = (
            size(settings.width, "宽度")?,
            size(settings.height, "高度")?,
        );
        let file = File::create(path)
            .map_err(|e| PlatformError::new(format!("无法创建 GIF 文件：{e}")))?;
        let mut encoder =
            Encoder::new(BufWriter::new(file), width, height, &[]).map_err(gif_err)?;
        encoder.set_repeat(Repeat::Infinite).map_err(gif_err)?;
        Ok(Self {
            encoder,
            width: settings.width,
            height: settings.height,
            frame_period: Duration::from_secs(1) / settings.frame_rate.max(1),
            canvas: None,
            pending: None,
            last_timestamp: Duration::ZERO,
        })
    }

    /// 写出等待中的那一帧，它一直显示到 `end`。
    fn flush_pending(&mut self, end: Duration) -> Result<(), PlatformError> {
        let Some(Pending { start, mut frame }) = self.pending.take() else {
            return Ok(());
        };
        frame.delay = delay(start, end);
        self.encoder.write_frame(&frame).map_err(gif_err)
    }
}

impl VideoWriter for GifWriter {
    fn write(&mut self, frame: &dyn VideoFrame, timestamp: Duration) -> Result<(), PlatformError> {
        let image = frame.to_image()?;
        if (image.width(), image.height()) != (self.width, self.height) {
            return Err(PlatformError::new("GIF：帧尺寸与视频尺寸不一致"));
        }
        self.last_timestamp = timestamp;
        let patch = match &self.canvas {
            None => Some((0, 0, image.width(), image.height())),
            Some(previous) => changed_rect(previous, &image),
        };
        // 与画布完全相同：不写新帧，上一帧继续显示。
        let Some((left, top, width, height)) = patch else {
            return Ok(());
        };
        self.flush_pending(timestamp)?;
        let mut rgba = crop_opaque(&image, left, top, width, height);
        let mut quantized =
            Frame::from_rgba_speed(width as u16, height as u16, &mut rgba, QUANTIZE_SPEED);
        quantized.left = left as u16;
        quantized.top = top as u16;
        quantized.dispose = DisposalMethod::Keep;
        self.pending = Some(Pending {
            start: timestamp,
            frame: quantized,
        });
        self.canvas = Some(image);
        Ok(())
    }

    fn write_audio(&mut self, _samples: &[f32]) -> Result<(), PlatformError> {
        Err(PlatformError::new("GIF 没有音轨"))
    }

    fn finish(mut self: Box<Self>) -> Result<(), PlatformError> {
        // 最后一帧显示 1 / 帧率 秒（与 `VideoWriter::write` 的约定一致）。
        let end = self.last_timestamp + self.frame_period;
        self.flush_pending(end)?;
        // 取出内部的 BufWriter 并 flush，写文件的错误在这里才暴露。
        let mut out = self.encoder.into_inner().map_err(gif_err)?;
        std::io::Write::flush(&mut out)
            .map_err(|e| PlatformError::new(format!("写入 GIF 文件失败：{e}")))
    }
}

fn gif_err(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::new(format!("GIF 编码失败：{e}"))
}

/// 从 `start` 到 `end` 的停留时间（1/100 秒）：用两端各自取整后的差，累计误差不会超过 1/100 秒。
fn delay(start: Duration, end: Duration) -> u16 {
    let centis = |t: Duration| (t.as_micros() as u64 + 5_000) / 10_000;
    (centis(end).saturating_sub(centis(start))).clamp(MIN_DELAY, MAX_DELAY) as u16
}

/// 两幅同尺寸图像不同的像素的外接矩形 `(left, top, width, height)`；完全相同返回 `None`。
fn changed_rect(a: &Image, b: &Image) -> Option<(u32, u32, u32, u32)> {
    let (width, height) = (a.width() as usize, a.height() as usize);
    let (pa, pb) = (a.rgba(), b.rgba());
    let row =
        |y: usize| pa[y * width * 4..(y + 1) * width * 4] != pb[y * width * 4..(y + 1) * width * 4];
    let top = (0..height).find(|&y| row(y))?;
    let bottom = (top..height).rfind(|&y| row(y))?;
    let (mut left, mut right) = (width, 0);
    for y in top..=bottom {
        let (ra, rb) = (
            &pa[y * width * 4..(y + 1) * width * 4],
            &pb[y * width * 4..(y + 1) * width * 4],
        );
        if let Some(first) = (0..width).find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            let last = (first..width).rfind(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4])?;
            left = left.min(first);
            right = right.max(last);
        }
    }
    Some((
        left as u32,
        top as u32,
        (right - left + 1) as u32,
        (bottom - top + 1) as u32,
    ))
}

/// 裁出矩形内的 RGBA 像素，alpha 一律置为不透明（屏幕画面没有透明度，避免量化器为透明保留颜色槽）。
fn crop_opaque(image: &Image, left: u32, top: u32, width: u32, height: u32) -> Vec<u8> {
    let stride = image.width() as usize * 4;
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in top..top + height {
        let start = y as usize * stride + left as usize * 4;
        out.extend_from_slice(&image.rgba()[start..start + width as usize * 4]);
    }
    for pixel in out.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bps_platform::VideoFormat;
    use std::any::Any;

    struct TestFrame(Image);
    impl VideoFrame for TestFrame {
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

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Image {
        Image::from_rgba(w, h, [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize))
    }

    fn with_block(mut base: Image, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) -> Image {
        let width = base.width();
        let mut px = std::mem::replace(&mut base, Image::from_rgba(0, 0, vec![])).into_rgba();
        for yy in y..y + h {
            for xx in x..x + w {
                let i = ((yy * width + xx) * 4) as usize;
                px[i..i + 3].copy_from_slice(&rgb);
            }
        }
        Image::from_rgba(width, px.len() as u32 / 4 / width, px)
    }

    struct Decoded {
        size: (u16, u16),
        /// 每个 GIF 帧的 (left, top, width, height, delay)。
        frames: Vec<(u16, u16, u16, u16, u16)>,
        /// 逐帧叠加后的最终画布 RGBA。
        canvas: Vec<u8>,
    }

    fn decode(path: &Path) -> Decoded {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(File::open(path).unwrap()).unwrap();
        let (w, h) = (decoder.width(), decoder.height());
        let mut canvas = vec![0u8; w as usize * h as usize * 4];
        let mut frames = vec![];
        while let Some(f) = decoder.read_next_frame().unwrap() {
            frames.push((f.left, f.top, f.width, f.height, f.delay));
            for y in 0..f.height as usize {
                for x in 0..f.width as usize {
                    let src = (y * f.width as usize + x) * 4;
                    let dst = ((f.top as usize + y) * w as usize + f.left as usize + x) * 4;
                    canvas[dst..dst + 4].copy_from_slice(&f.buffer[src..src + 4]);
                }
            }
        }
        Decoded {
            size: (w, h),
            frames,
            canvas,
        }
    }

    fn near(a: &[u8], b: [u8; 3]) -> bool {
        (0..3).all(|i| a[i].abs_diff(b[i]) <= 8)
    }

    fn write_all(name: &str, fps: u32, size: (u32, u32), frames: &[(u64, Image)]) -> Decoded {
        let path =
            std::env::temp_dir().join(format!("bps-gif-test-{name}-{}.gif", std::process::id()));
        let mut writer: Box<dyn VideoWriter> = Box::new(
            GifWriter::start(
                &path,
                &VideoSettings {
                    format: VideoFormat::Gif,
                    width: size.0,
                    height: size.1,
                    frame_rate: fps,
                    audio: None,
                    codec: bps_platform::VideoCodec::H264,
                    bitrate: 0,
                },
            )
            .unwrap(),
        );
        for (ms, image) in frames {
            writer
                .write(&TestFrame(image.clone()), Duration::from_millis(*ms))
                .unwrap();
        }
        writer.finish().unwrap();
        let decoded = decode(&path);
        std::fs::remove_file(&path).unwrap();
        decoded
    }

    #[test]
    fn unchanged_frames_extend_the_previous_frame_and_only_changes_are_written() {
        let red = solid(20, 16, [255, 0, 0]);
        let blue_block = with_block(red.clone(), 5, 4, 6, 3, [0, 0, 255]);
        let frames = [
            (0, red.clone()),
            (100, red.clone()),
            (200, red),
            (300, blue_block.clone()),
            (400, blue_block),
        ];

        let gif = write_all("delta", 10, (20, 16), &frames);

        assert_eq!(gif.size, (20, 16));
        // 第 1 帧整幅停留 300 ms；第 2 帧只有变化的 6×3 方块，停留 100 + 100（最后一帧 1/fps）。
        assert_eq!(gif.frames, [(0, 0, 20, 16, 30), (5, 4, 6, 3, 20)]);
        let at = |x: usize, y: usize| &gif.canvas[(y * 20 + x) * 4..][..4];
        assert!(near(at(0, 0), [255, 0, 0]));
        assert!(near(at(5, 4), [0, 0, 255]));
        assert!(near(at(10, 6), [0, 0, 255]));
        assert!(near(at(11, 6), [255, 0, 0]));
    }

    #[test]
    fn delays_follow_timestamps_without_accumulating_rounding_error() {
        // 15 fps：每帧 66.67 ms，取整成 6 或 7 厘秒，30 帧合计必须正好 200 厘秒。
        let frames: Vec<(u64, Image)> = (0..30u64)
            .map(|k| (k * 1000 / 15, solid(4, 4, [(k * 8) as u8, 0, 0])))
            .collect();

        let gif = write_all("delay", 15, (4, 4), &frames);

        assert_eq!(gif.frames.len(), 30);
        let total: u32 = gif.frames.iter().map(|f| u32::from(f.4)).sum();
        assert_eq!(total, 200);
        assert!(gif.frames.iter().all(|f| f.4 == 6 || f.4 == 7));
    }

    #[test]
    fn rejects_frames_of_wrong_size() {
        let path =
            std::env::temp_dir().join(format!("bps-gif-test-size-{}.gif", std::process::id()));
        let mut writer = GifWriter::start(
            &path,
            &VideoSettings {
                format: VideoFormat::Gif,
                width: 4,
                height: 4,
                frame_rate: 10,
                audio: None,
                codec: bps_platform::VideoCodec::H264,
                bitrate: 0,
            },
        )
        .unwrap();
        let result = writer.write(&TestFrame(solid(5, 4, [0, 0, 0])), Duration::ZERO);
        drop(writer);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }
}
