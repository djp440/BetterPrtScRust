//! 录屏真机冒烟，默认忽略：
//! - 实时画面上的遮罩：闪现不到 1 秒（只盖住一个 40×40 的小方块），检查它不出现在截图里、能收到输入；
//! - 录屏：用真实的节拍、Windows.Graphics.Capture 和 Media Foundation 录两段约 1 秒的小区域视频
//!   到临时目录，用 Media Foundation 解码检查帧数、时间戳、尺寸和画面，然后删除。
//!
//! 不启动常驻实例、不碰剪贴板、不注册热键。会显示窗口，运行前要拿真机桌面锁：
//! `cargo test -p bps-platform-windows --test video_smoke -- --ignored --nocapture --test-threads=1`
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use bps_platform::{
    Backdrop, Color, Display, FileSystem, FrameSource, Image, NewFile, OverlayInput, OverlayItem,
    OverlayKey, OverlayScene, PlatformEvent, Rect, SelectionOverlay, StreamFrame, StreamOptions,
    Ticker, VideoCodec, VideoEncoder, VideoFormat, VideoFrame, VideoSettings, VideoSource,
    VideoTarget,
};
use bps_platform_windows::{
    DxgiFrameSource, EventLoop, MfVideoEncoder, WgcVideoSource, WindowsFileSystem, dpi,
    undecodable_video_codecs, unsupported_video_codecs,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFSample, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video,
    MFSTARTUP_LITE, MFShutdown, MFStartup, MFVideoFormat_AV1, MFVideoFormat_H264,
    MFVideoFormat_HEVC, MFVideoFormat_RGB32,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowDisplayAffinity, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    PostMessageW, WDA_EXCLUDEFROMCAPTURE, WM_KEYDOWN,
};
use windows::core::{GUID, HSTRING, PCWSTR, w};

const MAGENTA: [u8; 4] = [255, 0, 255, 255];

/// 本进程的某个类名的窗口。
fn own_window(class: PCWSTR) -> Option<HWND> {
    let pid = unsafe { GetCurrentProcessId() };
    let mut after = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, class, None) }.ok()?;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == pid {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
}

#[test]
#[ignore = "真机冒烟：闪现实时遮罩"]
fn live_overlay_is_excluded_from_capture_and_delivers_input() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let mut overlay = event_loop.selection_overlay();
    let frames = DxgiFrameSource::new();

    let started = Instant::now();
    let displays = overlay.open(Backdrop::Live).unwrap();
    let primary = displays.iter().find(|d| d.is_primary).unwrap().clone();
    let marker = Rect::new(primary.bounds.x + 100, primary.bounds.y + 100, 40, 40);
    overlay
        .render(&OverlayScene {
            items: vec![OverlayItem::Fill {
                rect: marker,
                color: Color::rgba(255, 0, 255, 255),
            }],
            ..OverlayScene::default()
        })
        .unwrap();
    eprintln!("打开实时遮罩到显示完成：{:?}", started.elapsed());

    let hwnd = own_window(w!("BetterPrtSc.LiveOverlay")).expect("应已创建实时遮罩窗口");
    assert!(
        unsafe { IsWindowVisible(hwnd) }.as_bool(),
        "实时遮罩应已显示"
    );
    let mut affinity = 0;
    unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.unwrap();
    assert_eq!(
        affinity, WDA_EXCLUDEFROMCAPTURE.0,
        "实时遮罩应排除在屏幕捕获之外"
    );
    let shot = frames.capture_display(primary.id).unwrap();
    assert_ne!(
        shot.pixel(120, 120),
        MAGENTA,
        "截图里不应出现遮罩上画的方块"
    );

    unsafe {
        PostMessageW(
            Some(hwnd),
            WM_KEYDOWN,
            WPARAM(usize::from(VK_ESCAPE.0)),
            LPARAM(0),
        )
    }
    .unwrap();
    let event = event_loop.wait_event(Duration::from_millis(500));
    overlay.close();
    assert!(
        matches!(
            event,
            Some(PlatformEvent::OverlayInput(OverlayInput::KeyPressed {
                key: OverlayKey::Escape,
                ..
            }))
        ),
        "{event:?}"
    );
    assert!(
        !unsafe { IsWindow(Some(hwnd)) }.as_bool(),
        "关闭后实时遮罩窗口应已销毁"
    );
}

#[test]
#[ignore = "真机冒烟：录两段约 1 秒的小区域视频"]
fn records_region_to_mp4_with_constant_frame_rate() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("bps-video-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cleanup = RemoveDirOnDrop(dir.clone());

    // 384×256（走硬件编码器）、30 fps、不带指针：画面与 Desktop Duplication 截图对比。
    let region = Rect::new(primary.bounds.x + 64, primary.bounds.y + 64, 384, 256);
    let recorded = record(
        &event_loop,
        &primary,
        &dir,
        "a",
        Take {
            region,
            show_cursor: false,
            fps: 30,
            duration: Duration::from_secs(1),
            codec: VideoCodec::H264,
        },
    );
    let capture_diff = mean_difference(&recorded.first_image, &recorded.screenshot);
    eprintln!(
        "录制 {} 帧，用时 {:?}；首帧与截图的平均差 {capture_diff:.2}",
        recorded.written, recorded.elapsed
    );
    assert!(
        capture_diff < 12.0,
        "视频源的画面应与截图一致（位置、颜色）"
    );
    let video = decode(&recorded.path);
    eprintln!(
        "解码：{} × {}，{:?} fps，{} 帧，最后时间戳 {:?}",
        video.width,
        video.height,
        video.frame_rate,
        video.timestamps.len(),
        video.timestamps.last()
    );
    assert_eq!((video.width, video.height), (384, 256));
    assert_eq!(video.subtype, MFVideoFormat_H264);
    assert_eq!(video.frame_rate, (30, 1));
    assert_eq!(video.timestamps.len() as u64, recorded.written);
    for (k, t) in video.timestamps.iter().enumerate() {
        let expected = k as i64 * 10_000_000 / 30;
        assert!(
            (t - expected).abs() <= 1,
            "第 {k} 帧时间戳 {t}，应为 {expected}"
        );
    }
    let encode_diff = mean_difference(&video.first_image, &recorded.first_image);
    eprintln!("解码后的首帧与编码前的平均差 {encode_diff:.2}");
    assert!(
        encode_diff < 16.0,
        "解码出的画面应与编码前一致（方向、颜色）"
    );

    // MP4 允许的最小区域（34×34，H.264 编码器要求宽高都大于 32）、10 fps、带指针。
    let tiny = Rect::new(primary.bounds.x + 8, primary.bounds.y + 8, 34, 34);
    let recorded = record(
        &event_loop,
        &primary,
        &dir,
        "b",
        Take {
            region: tiny,
            show_cursor: true,
            fps: 10,
            duration: Duration::from_millis(500),
            codec: VideoCodec::H264,
        },
    );
    let video = decode(&recorded.path);
    eprintln!(
        "小区域：录制 {} 帧，解码 {} × {}，{:?} fps，{} 帧",
        recorded.written,
        video.width,
        video.height,
        video.frame_rate,
        video.timestamps.len()
    );
    assert_eq!((video.width, video.height), (34, 34));
    assert_eq!(video.frame_rate, (10, 1));
    assert_eq!(video.timestamps.len() as u64, recorded.written);
    drop(cleanup);
    assert!(!dir.exists(), "应已删除录出的文件");
}

#[test]
#[ignore = "真机冒烟：用这台电脑支持的每种编码录约 1 秒的视频"]
fn records_with_every_supported_codec() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let primary = DxgiFrameSource::new()
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("bps-codec-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = RemoveDirOnDrop(dir.clone());

    let unsupported = unsupported_video_codecs();
    let undecodable = undecodable_video_codecs();
    eprintln!("不支持的编码：{unsupported:?}；系统不能解码：{undecodable:?}");
    assert!(
        !undecodable.contains(&VideoCodec::H264),
        "H.264 系统总能解码"
    );
    for codec in VideoCodec::ALL {
        if unsupported.iter().any(|(c, _)| *c == codec) {
            continue;
        }
        let region = Rect::new(primary.bounds.x + 64, primary.bounds.y + 64, 384, 256);
        let recorded = record(
            &event_loop,
            &primary,
            &dir,
            &format!("{codec:?}"),
            Take {
                region,
                show_cursor: false,
                fps: 30,
                duration: Duration::from_secs(1),
                codec,
            },
        );
        let size = std::fs::metadata(&recorded.path).unwrap().len();
        let expected = match codec {
            VideoCodec::H264 => MFVideoFormat_H264,
            VideoCodec::H265 => MFVideoFormat_HEVC,
            VideoCodec::Av1 => MFVideoFormat_AV1,
        };
        if undecodable.contains(&codec) {
            // 没有解码器时只能确认文件里是这种编码（Source Reader 读容器不需要解码器）。
            assert_eq!(native_subtype(&recorded.path), expected, "{codec:?}");
            eprintln!("{}：{size} 字节；系统没有解码器，不检查画面", codec.label());
            continue;
        }
        let video = decode(&recorded.path);
        let diff = mean_difference(&video.first_image, &recorded.first_image);
        eprintln!(
            "{}：{} 字节，解码 {} 帧，首帧平均差 {diff:.2}",
            codec.label(),
            size,
            video.timestamps.len()
        );
        assert_eq!(video.subtype, expected, "{codec:?}");
        assert_eq!((video.width, video.height), (384, 256));
        assert_eq!(video.timestamps.len() as u64, recorded.written, "{codec:?}");
        assert!(diff < 16.0, "{codec:?} 解码出的画面应与编码前一致");
    }
}

struct Recorded {
    path: PathBuf,
    written: u64,
    elapsed: Duration,
    first_image: Image,
    /// 拿到第一帧的同时用 Desktop Duplication 截下的同一块区域。
    screenshot: Image,
}

/// 录一段视频的参数。
struct Take {
    region: Rect,
    show_cursor: bool,
    fps: u32,
    duration: Duration,
    codec: VideoCodec,
}

/// 像 `core` 一样录一段视频：节拍到时按单调时间补齐帧，写进 `.part` 临时文件，最后改名。
fn record(
    event_loop: &EventLoop,
    display: &Display,
    dir: &Path,
    name: &str,
    take: Take,
) -> Recorded {
    let Take {
        region,
        show_cursor,
        fps,
        duration,
        codec,
    } = take;
    let mut file_system = WindowsFileSystem::new();
    let temp = dir.join(format!("{name}.mp4.part"));
    assert_eq!(
        file_system.create_new(&temp, &[]).unwrap(),
        NewFile::Created
    );
    let mut stream = WgcVideoSource::new()
        .open(VideoTarget::Region(region), StreamOptions { show_cursor })
        .unwrap();
    let mut writer = MfVideoEncoder::new()
        .start(
            &temp,
            &VideoSettings {
                format: VideoFormat::Mp4,
                width: region.width,
                height: region.height,
                frame_rate: fps,
                codec,
                bitrate: 4_000_000,
                audio: None,
            },
        )
        .unwrap();
    let mut ticker = event_loop.ticker();
    ticker.start(Duration::from_secs(1) / fps).unwrap();
    let started = Instant::now();
    let due =
        |elapsed: Duration| (elapsed.as_nanos() * u128::from(fps)).div_ceil(1_000_000_000) as u64;
    let mut latest: Option<Rc<dyn VideoFrame>> = None;
    let mut first_image = None;
    let mut screenshot = None;
    let mut written = 0;
    let mut catch_up = |target: u64, latest: &mut Option<Rc<dyn VideoFrame>>| {
        match stream.latest().unwrap() {
            StreamFrame::Frame(frame) => *latest = Some(frame),
            StreamFrame::Pending => {}
            StreamFrame::Ended => panic!("录制目标不应消失"),
        }
        let Some(frame) = latest.as_ref() else {
            return;
        };
        if first_image.is_none() {
            first_image = Some(frame.to_image().unwrap());
            let shot = DxgiFrameSource::new().capture_display(display.id).unwrap();
            screenshot = Some(crop(&shot, region, display.bounds));
        }
        while written < target {
            let timestamp = Duration::from_nanos(written * 1_000_000_000 / u64::from(fps));
            writer.write(frame.as_ref(), timestamp).unwrap();
            written += 1;
        }
    };
    while started.elapsed() < duration {
        if let Some(PlatformEvent::Tick) = event_loop.wait_event(Duration::from_millis(200)) {
            catch_up(due(started.elapsed()), &mut latest);
        }
    }
    let elapsed = started.elapsed();
    catch_up(due(elapsed).max(1), &mut latest);
    ticker.stop();
    drop(stream);
    writer.finish().unwrap();
    assert!(written > 0, "应收到画面");
    let path = dir.join(format!("{name}.mp4"));
    assert_eq!(
        file_system.rename_new(&temp, &path).unwrap(),
        NewFile::Created,
        "写完后应能立即改名"
    );
    Recorded {
        path,
        written,
        elapsed,
        first_image: first_image.unwrap(),
        screenshot: screenshot.unwrap(),
    }
}

struct Decoded {
    /// 视频流的编码（`MFVideoFormat_*`）。
    subtype: GUID,
    width: u32,
    height: u32,
    frame_rate: (u32, u32),
    /// 各帧的时间戳（100 纳秒）。
    timestamps: Vec<i64>,
    first_image: Image,
}

/// MP4 里视频流的编码（`MFVideoFormat_*`），不解码。
fn native_subtype(path: &Path) -> GUID {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).unwrap();
        let reader =
            MFCreateSourceReaderFromURL(&HSTRING::from(path.to_str().unwrap()), None).unwrap();
        let subtype = reader
            .GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, 0)
            .unwrap()
            .GetGUID(&MF_MT_SUBTYPE)
            .unwrap();
        drop(reader);
        MFShutdown().unwrap();
        subtype
    }
}

/// 用 Media Foundation 的 Source Reader 解码 MP4，输出 RGB32。
fn decode(path: &Path) -> Decoded {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).unwrap();
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 1).unwrap();
        let attributes = attributes.unwrap();
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .unwrap();
        let reader =
            MFCreateSourceReaderFromURL(&HSTRING::from(path.to_str().unwrap()), &attributes)
                .unwrap();
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let native = reader.GetNativeMediaType(stream, 0).unwrap();
        let subtype = native.GetGUID(&MF_MT_SUBTYPE).unwrap();
        let size = native.GetUINT64(&MF_MT_FRAME_SIZE).unwrap();
        let rate = native.GetUINT64(&MF_MT_FRAME_RATE).unwrap();
        let output = MFCreateMediaType().unwrap();
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .unwrap();
        output
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .unwrap();
        reader
            .SetCurrentMediaType(stream, None, &output)
            .expect("系统应能解码这种视频（系统播放器同样用它）");
        // RGB 的行跨度为负表示自下而上；没有这个属性时按 RGB 的默认（自下而上）处理。
        let stride = reader
            .GetCurrentMediaType(stream)
            .unwrap()
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map_or(-(((size >> 32) as i32) * 4), |s| s as i32);
        eprintln!("解码输出的行跨度 {stride}");
        let (width, height) = ((size >> 32) as u32, size as u32);
        let mut timestamps = Vec::new();
        let mut first_image = None;
        loop {
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample: Option<IMFSample> = None;
            reader
                .ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
                .unwrap();
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else {
                continue;
            };
            timestamps.push(timestamp);
            if first_image.is_none() {
                first_image = Some(read_rgb32(&sample, width, height, stride));
            }
        }
        drop(reader);
        MFShutdown().unwrap();
        Decoded {
            subtype,
            width,
            height,
            frame_rate: ((rate >> 32) as u32, rate as u32),
            timestamps,
            first_image: first_image.expect("视频里应至少有一帧"),
        }
    }
}

unsafe fn read_rgb32(sample: &IMFSample, width: u32, height: u32, stride: i32) -> Image {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().unwrap();
        let mut data = std::ptr::null_mut();
        let mut length = 0;
        buffer.Lock(&mut data, None, Some(&mut length)).unwrap();
        let bytes = std::slice::from_raw_parts(data, length as usize);
        let pitch = stride.unsigned_abs() as usize;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height as usize {
            let row = if stride < 0 {
                height as usize - 1 - y
            } else {
                y
            };
            let row = &bytes[row * pitch..row * pitch + width as usize * 4];
            for p in row.chunks_exact(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        }
        buffer.Unlock().unwrap();
        Image::from_rgba(width, height, rgba)
    }
}

/// 显示器截图中 `region`（虚拟桌面坐标）那一块。
fn crop(image: &Image, region: Rect, display: Rect) -> Image {
    let mut rgba = Vec::with_capacity((region.width * region.height * 4) as usize);
    for y in 0..region.height {
        for x in 0..region.width {
            let pixel = image.pixel(
                (region.x - display.x) as u32 + x,
                (region.y - display.y) as u32 + y,
            );
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Image::from_rgba(region.width, region.height, rgba)
}

/// 两张同尺寸图像 RGB 通道的平均绝对差。
fn mean_difference(a: &Image, b: &Image) -> f64 {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let total: u64 = a
        .rgba()
        .chunks_exact(4)
        .zip(b.rgba().chunks_exact(4))
        .map(|(p, q)| (0..3).map(|i| u64::from(p[i].abs_diff(q[i]))).sum::<u64>())
        .sum();
    total as f64 / (f64::from(a.width()) * f64::from(a.height()) * 3.0)
}

/// 测试结束（包括失败）时删除本次建的临时目录。
struct RemoveDirOnDrop(PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "真机冒烟：闪现录屏控制条和区域边框"]
fn recording_toolbar_and_border_are_excluded_from_capture_and_do_not_steal_focus() {
    use bps_platform::{RecordingToolbar, RecordingToolbarView, RegionBorder};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let mut toolbar = event_loop.recording_toolbar();
    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let region = Rect::new(primary.bounds.x + 300, primary.bounds.y + 300, 300, 200);
    let bounds = Rect::new(region.x + 100, region.y + 200 + 12, 200, 40);
    let red = Color::rgba(240, 40, 40, 255);
    let view = RecordingToolbarView {
        bounds,
        items: vec![OverlayItem::Fill {
            rect: bounds,
            color: red,
        }],
        border: Some(RegionBorder {
            rect: region,
            width: 4,
            color: red,
        }),
    };
    let foreground = unsafe { GetForegroundWindow() };
    toolbar.show(&view).unwrap();
    toolbar.show(&view).unwrap();

    let bar = own_window(w!("BetterPrtSc.LiveOverlay")).expect("应已创建控制条窗口");
    let edge = own_window(w!("BetterPrtSc.RegionBorder")).expect("应已创建边框窗口");
    for hwnd in [bar, edge] {
        assert!(unsafe { IsWindowVisible(hwnd) }.as_bool());
        let mut affinity = 0;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.unwrap();
        assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE.0);
    }
    assert_eq!(
        unsafe { GetForegroundWindow() },
        foreground,
        "控制条不能抢前台"
    );
    let shot = frames.capture_display(primary.id).unwrap();
    let local =
        |x: i32, y: i32| shot.pixel((x - primary.bounds.x) as u32, (y - primary.bounds.y) as u32);
    let red = [240, 40, 40, 255];
    assert_ne!(local(region.x - 2, region.y + 100), red, "截图里不应有边框");
    assert_ne!(
        local(bounds.x + 100, bounds.y + 20),
        red,
        "截图里不应有控制条"
    );

    toolbar.hide();
    assert!(!unsafe { IsWindow(Some(bar)) }.as_bool());
    assert!(!unsafe { IsWindow(Some(edge)) }.as_bool());
}
