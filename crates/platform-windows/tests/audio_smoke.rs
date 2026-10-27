//! 系统声音真机冒烟，默认忽略：用另一个进程播放系统自带的 `Alarm01.wav`，同时
//! - 回环采集默认输出设备，检查采集到了声音、采集时间递增且贴近此刻；
//! - 像 `core` 一样录一段约 2 秒带音轨的小区域 MP4 到临时目录，用 Media Foundation 解码检查
//!   音轨是 AAC、有声音、与画面等长，然后删除。
//!
//! 会发出声音。需要可用的音频输出设备：
//! `cargo test -p bps-platform-windows --test audio_smoke -- --ignored --nocapture --test-threads=1`
#![cfg(windows)]

use std::path::Path;
use std::process::{Child, Command};
use std::rc::Rc;
use std::time::{Duration, Instant};

use bps_platform::{
    AudioSource, Clock, FrameSource, PlatformEvent, Rect, StreamFrame, StreamOptions, Ticker,
    VideoCodec, VideoEncoder, VideoFormat, VideoFrame, VideoSettings, VideoSource, VideoTarget,
};
use bps_platform_windows::{
    DxgiFrameSource, EventLoop, MfVideoEncoder, WasapiAudioSource, WgcVideoSource, WindowsClock,
    dpi, microphones,
};
use windows::Win32::Media::MediaFoundation::{
    IMFSample, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFSTARTUP_LITE, MFShutdown,
    MFStartup,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

/// 在另一个进程里播放一段几秒长的系统提示音。
fn play_alarm() -> Child {
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(New-Object Media.SoundPlayer 'C:\\Windows\\Media\\Alarm01.wav').PlaySync()",
        ])
        .spawn()
        .expect("启动 PowerShell 播放提示音失败")
}

#[test]
#[ignore = "真机冒烟：会发出声音"]
fn loopback_captures_playing_sound_with_current_timestamps() {
    let clock = WindowsClock::new();
    let mut source = WasapiAudioSource::new();
    let format = source.format();
    let mut stream = source.open_system_audio().unwrap();
    assert_eq!((format.sample_rate, format.channels), (48_000, 2));
    let mut player = play_alarm();
    let started = Instant::now();
    let mut chunks = Vec::new();
    while started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(30));
        let read = stream.read().unwrap();
        let now = clock.monotonic();
        for chunk in read {
            assert!(chunk.captured_at <= now);
            assert!(
                now - chunk.captured_at < Duration::from_millis(500),
                "采集时间应贴近此刻：晚了 {:?}",
                now - chunk.captured_at
            );
            assert_eq!(chunk.samples.len() % 2, 0);
            chunks.push(chunk);
        }
    }
    let _ = player.kill();
    let _ = player.wait();
    assert!(
        chunks
            .windows(2)
            .all(|w| w[0].captured_at <= w[1].captured_at),
        "采集时间应递增"
    );
    let peak = chunks
        .iter()
        .flat_map(|c| c.samples.iter())
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    eprintln!("采集到 {} 段声音，峰值 {peak}", chunks.len());
    assert!(peak > 0.01, "播放提示音期间应采集到声音（峰值 {peak}）");
}

#[test]
#[ignore = "真机冒烟：打开麦克风采集约 1 秒（需要有麦克风）"]
fn microphone_captures_with_current_timestamps() {
    let devices = microphones().unwrap();
    eprintln!("麦克风：{devices:?}");
    assert!(!devices.is_empty(), "这台电脑应至少有一个麦克风");
    let clock = WindowsClock::new();
    let mut source = WasapiAudioSource::new();
    // 系统默认设备和按设备标识打开都要能用。
    for device in [None, Some(devices[0].id.as_str())] {
        let mut stream = source.open_microphone(device).unwrap();
        let started = Instant::now();
        let mut frames = 0;
        let mut previous = Duration::ZERO;
        while started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(30));
            let now = clock.monotonic();
            for chunk in stream.read().unwrap() {
                assert!(
                    chunk.captured_at <= now
                        && now - chunk.captured_at < Duration::from_millis(500)
                );
                assert!(chunk.captured_at >= previous, "采集时间应递增");
                previous = chunk.captured_at;
                assert_eq!(chunk.samples.len() % 2, 0);
                frames += chunk.samples.len() / 2;
            }
        }
        eprintln!("{device:?}：1 秒采集到 {frames} 帧");
        // 麦克风即使没人说话也持续产生数据（与回环不同）。
        assert!(
            frames > 30_000,
            "{device:?} 应持续产生声音数据，只有 {frames} 帧"
        );
    }
    assert!(
        source.open_microphone(Some("不存在的设备")).is_err(),
        "不存在的设备应打开失败（core 据此回退到默认麦克风）"
    );
}

#[test]
#[ignore = "真机冒烟：会发出声音、录一段小视频"]
fn mp4_with_system_audio_has_aac_track_as_long_as_video() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let display = DxgiFrameSource::new().displays().unwrap()[0].clone();
    let region = Rect::new(display.bounds.x, display.bounds.y, 64, 64);
    let dir = std::env::temp_dir().join(format!("bps-audio-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("audio.mp4");

    let clock = WindowsClock::new();
    let fps = 30;
    let mut source = WasapiAudioSource::new();
    let format = source.format();
    let mut audio = source.open_system_audio().unwrap();
    let mut video = WgcVideoSource::new()
        .open(
            VideoTarget::Region(region),
            StreamOptions { show_cursor: false },
        )
        .unwrap();
    let mut writer = MfVideoEncoder::new()
        .start(
            &path,
            &VideoSettings {
                format: VideoFormat::Mp4,
                width: region.width,
                height: region.height,
                frame_rate: fps,
                codec: VideoCodec::H264,
                bitrate: 2_000_000,
                audio: Some(format),
            },
        )
        .unwrap();
    let mut player = play_alarm();
    let mut ticker = event_loop.ticker();
    ticker.start(Duration::from_secs(1) / fps).unwrap();
    let started = clock.monotonic();
    let rate = u128::from(format.sample_rate);
    let channels = usize::from(format.channels);
    let frames_at =
        |at: Duration| (at.saturating_sub(started).as_nanos() * rate / 1_000_000_000) as usize;
    let mut audio_written = 0usize;
    let mut latest: Option<Rc<dyn VideoFrame>> = None;
    let mut written = 0u64;
    let duration = Duration::from_secs(2);
    loop {
        let now = clock.monotonic();
        for chunk in audio.read().unwrap() {
            // 与 `core` 相同：按采集时间放置，相差不超过 30 毫秒时直接接上；
            // 更晚时前面补静音，更早时丢掉重叠的部分。
            let target = frames_at(chunk.captured_at);
            let tolerance = format.sample_rate as usize * 30 / 1000;
            let mut samples = &chunk.samples[..];
            if target > audio_written + tolerance {
                writer
                    .write_audio(&vec![0.0; (target - audio_written) * channels])
                    .unwrap();
                audio_written = target;
            } else if target + tolerance < audio_written {
                samples = &samples[((audio_written - target) * channels).min(samples.len())..];
            }
            if !samples.is_empty() {
                writer.write_audio(samples).unwrap();
                audio_written += samples.len() / channels;
            }
        }
        if let StreamFrame::Frame(frame) = video.latest().unwrap() {
            latest = Some(frame);
        }
        let elapsed = now - started;
        let due = (elapsed.as_nanos() * u128::from(fps)).div_ceil(1_000_000_000) as u64;
        if let Some(frame) = latest.as_ref() {
            while written < due {
                let timestamp = Duration::from_nanos(written * 1_000_000_000 / u64::from(fps));
                writer.write(frame.as_ref(), timestamp).unwrap();
                written += 1;
            }
        }
        if elapsed >= duration {
            let end = frames_at(now);
            if end > audio_written {
                writer
                    .write_audio(&vec![0.0; (end - audio_written) * channels])
                    .unwrap();
            }
            break;
        }
        while !matches!(
            event_loop.wait_event(Duration::from_millis(200)),
            Some(PlatformEvent::Tick)
        ) {}
    }
    ticker.stop();
    drop(video);
    drop(audio);
    let _ = player.kill();
    let _ = player.wait();
    writer.finish().unwrap();
    assert!(written > 0, "应收到画面");

    let decoded = decode(&path);
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "视频 {:?}、音频 {:?}、峰值 {}",
        decoded.video_duration, decoded.audio_duration, decoded.audio_peak
    );
    assert!(
        decoded.audio_peak > 300,
        "音轨里应有声音（峰值 {}）",
        decoded.audio_peak
    );
    let difference = decoded.video_duration.abs_diff(decoded.audio_duration);
    assert!(
        difference < Duration::from_millis(100),
        "音轨应与画面等长：视频 {:?}，音频 {:?}",
        decoded.video_duration,
        decoded.audio_duration
    );
}

struct Decoded {
    video_duration: Duration,
    audio_duration: Duration,
    /// 解码出的 16 位样本的最大绝对值。
    audio_peak: i32,
}

/// 用 Source Reader 读 MP4：视频只取时间戳，音频解码成 16 位 PCM。
fn decode(path: &Path) -> Decoded {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).unwrap();
        let reader =
            MFCreateSourceReaderFromURL(&HSTRING::from(path.to_str().unwrap()), None).unwrap();
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let native = reader.GetNativeMediaType(audio, 0).expect("MP4 里应有音轨");
        assert_eq!(native.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFAudioFormat_AAC);
        let output = MFCreateMediaType().unwrap();
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .unwrap();
        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).unwrap();
        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).unwrap();
        reader.SetCurrentMediaType(audio, None, &output).unwrap();

        let end_of = |stream: u32, peak: &mut i32| -> i64 {
            let mut end = 0;
            loop {
                let (mut flags, mut timestamp) = (0, 0);
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
                    return end;
                }
                let Some(sample) = sample else { continue };
                end = timestamp + sample.GetSampleDuration().unwrap_or(0);
                if stream == audio {
                    let buffer = sample.ConvertToContiguousBuffer().unwrap();
                    let (mut data, mut length) = (std::ptr::null_mut(), 0);
                    buffer.Lock(&mut data, None, Some(&mut length)).unwrap();
                    let bytes = std::slice::from_raw_parts(data, length as usize);
                    for pair in bytes.chunks_exact(2) {
                        *peak =
                            (*peak).max(i32::from(i16::from_le_bytes([pair[0], pair[1]])).abs());
                    }
                    buffer.Unlock().unwrap();
                }
            }
        };
        let mut peak = 0;
        let audio_end = end_of(audio, &mut peak);
        let video_end = end_of(video, &mut 0);
        drop(reader);
        MFShutdown().unwrap();
        let hundred_nanos = |t: i64| Duration::from_nanos(t as u64 * 100);
        Decoded {
            video_duration: hundred_nanos(video_end),
            audio_duration: hundred_nanos(audio_end),
            audio_peak: peak,
        }
    }
}
