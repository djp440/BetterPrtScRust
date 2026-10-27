//! 录屏的系统声音（#29）：MP4 默认带一条系统声音的音轨，按采集时间与画面对齐；暂停期间的声音
//! 不录；没有声音的时段是静音。全局设置 `record_system_audio` 可以关掉，每条绑定可以单独覆盖。
//! 关掉声音时 MP4 仍有一条静音的音轨，录制中可以在控制条上打开（#30）。
//!
//! 时间由测试推进：`advance_clock` 让单调时间前进，`tick` 生成节拍事件；`play_audio` 让系统
//! 从此刻起播放一段声音。假音频源每毫秒一个样本（`FAKE_AUDIO_FORMAT`）。

use std::time::Duration;

use bps_core::{Config, ResidentInstance};
use bps_platform::{Hotkey, Key, Modifiers, PlatformEvent, Point, Rect};
use bps_platform_fake::decode::{self, FakeVideo};
use bps_platform_fake::{FAKE_AUDIO_FORMAT, FakeDisplay, FakePlatform};

const ALT_PRINT_SCREEN: Hotkey = Hotkey::new(
    Modifiers {
        ctrl: false,
        alt: true,
        shift: false,
        win: false,
    },
    Key::PrintScreen,
);

const REGION: Rect = Rect::new(100, 100, 40, 34);

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 640, 480)).primary())
        .with_cursor_at(Point::new(0, 0))
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 用 `config`（之后追加一条 Alt+PrtSc 区域录屏绑定，`overrides` 是绑定额外的配置行）
/// 框选 [`REGION`] 后正在录制的常驻实例。
fn recording_with(fake: &FakePlatform, globals: &str, overrides: &str) -> ResidentInstance {
    let config = Config::parse(&format!(
        "{globals}\n[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"region\"\nfunction = \"record\"\n{overrides}\n"
    ))
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(REGION));
    assert!(fake.capturing_video(), "确定区域后应开始录制");
    instance
}

fn recording(fake: &FakePlatform) -> ResidentInstance {
    recording_with(fake, "", "")
}

/// 按节拍的周期推进时间，直到过去 `duration`；每到一个节拍就交给常驻实例。
fn record_for(fake: &FakePlatform, instance: &mut ResidentInstance, duration: Duration) {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        let period = fake.ticker_period().expect("录制期间节拍应在运行");
        let step = period.min(duration - elapsed);
        fake.advance_clock(step);
        elapsed += step;
        instance.handle(fake.tick().unwrap()).unwrap();
    }
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn stop_and_read_video(fake: &FakePlatform, instance: &mut ResidentInstance) -> FakeVideo {
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    let path = fake.clipboard().files().expect("剪贴板应附带录屏文件")[0].clone();
    decode::video(&fake.file(&path).expect("录屏文件应存在"))
}

/// 音轨的样本，每段 `(电平, 毫秒数)` 首尾相接。
fn segments(parts: &[(f32, usize)]) -> Vec<f32> {
    parts
        .iter()
        .flat_map(|&(level, count)| std::iter::repeat_n(level, count))
        .collect()
}

#[test]
fn mp4_recording_has_system_audio_aligned_with_video() {
    let fake = desktop();
    let mut instance = recording(&fake);
    assert!(fake.capturing_audio(), "MP4 录屏默认采集系统声音");

    record_for(&fake, &mut instance, ms(200));
    fake.play_audio(0.5, ms(300));
    record_for(&fake, &mut instance, ms(300));
    record_for(&fake, &mut instance, ms(500));
    let video = stop_and_read_video(&fake, &mut instance);

    let audio = video.audio.expect("MP4 应带音轨");
    assert_eq!(audio.format, FAKE_AUDIO_FORMAT);
    assert_eq!(
        audio.samples,
        segments(&[(0.0, 200), (0.5, 300), (0.0, 500)]),
        "声音落在播放时刻；没有声音的时段是静音，音轨与 1 秒的画面等长"
    );
    assert!(!fake.capturing_audio(), "录完停止采集");
}

#[test]
fn audio_paused_with_video_leaves_no_gap() {
    let fake = desktop();
    let mut instance = recording(&fake);
    fake.play_audio(0.5, ms(200));
    record_for(&fake, &mut instance, ms(200));

    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    fake.play_audio(0.9, ms(300));
    record_for(&fake, &mut instance, ms(300));
    feed(&mut instance, fake.click_recording_toolbar("继续"));
    fake.play_audio(0.3, ms(200));
    record_for(&fake, &mut instance, ms(200));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("MP4 应带音轨").samples,
        segments(&[(0.5, 200), (0.3, 200)]),
        "暂停期间的声音不录，继续后的声音紧接着暂停前的"
    );
}

#[test]
fn audio_captured_before_pause_but_read_after_is_kept_up_to_the_pause() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, ms(200));

    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    // 平台在暂停之后才交出一段从暂停前 50 毫秒开始的声音。
    fake.play_audio_since(0.5, ms(50), ms(100));
    record_for(&fake, &mut instance, ms(100));
    feed(&mut instance, fake.click_recording_toolbar("继续"));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("MP4 应带音轨").samples,
        segments(&[(0.0, 150), (0.5, 50), (0.0, 100)]),
        "暂停前的部分照常录入，暂停之后的部分不录"
    );
}

#[test]
fn overlapping_audio_is_trimmed_to_keep_sync() {
    let fake = desktop();
    let mut instance = recording(&fake);
    fake.play_audio(0.5, ms(300));
    record_for(&fake, &mut instance, ms(100));
    // 平台报告的下一段声音与已经收到的重叠了 200 毫秒（采集时钟的偏差）。
    fake.play_audio(0.7, ms(300));
    record_for(&fake, &mut instance, ms(400));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("MP4 应带音轨").samples,
        segments(&[(0.5, 300), (0.7, 100), (0.0, 100)]),
        "按采集时间对齐：重叠的部分丢掉，而不是把后面的声音整体推迟"
    );
}

#[test]
fn record_system_audio_off_records_silent_video() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "record_system_audio = false", "");
    assert!(!fake.capturing_audio());
    fake.play_audio(0.5, ms(300));
    record_for(&fake, &mut instance, ms(300));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("音轨留着，录制中可以打开声音").samples,
        segments(&[(0.0, 300)]),
        "关掉后音轨是静音"
    );
    assert_eq!(video.frames.len(), 9);
}

#[test]
fn binding_overrides_global_record_system_audio() {
    let fake = desktop();
    let mut instance = recording_with(
        &fake,
        "record_system_audio = false",
        "record_system_audio = true",
    );
    fake.play_audio(0.5, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.audio.unwrap().samples, segments(&[(0.5, 100)]));
}

#[test]
fn gif_has_no_audio() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "", "output_format = \"gif\"");
    assert!(!fake.capturing_audio(), "GIF 没有音轨，不采集声音");
    record_for(&fake, &mut instance, ms(200));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.audio, None);
}

#[test]
fn audio_encoder_failure_falls_back_to_silent_video_and_notifies() {
    let fake = desktop().with_audio_encoder_failing("没有可用的 AAC 编码器");
    let mut instance = recording(&fake);

    let notice = &fake.notifications()[0];
    assert_eq!(notice.title, "录屏没有声音");
    assert!(
        notice.body.contains("没有可用的 AAC 编码器"),
        "{}",
        notice.body
    );
    assert!(!fake.capturing_audio(), "退回无声视频后不再采集声音");
    record_for(&fake, &mut instance, ms(300));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.audio, None);
    assert_eq!(video.frames.len(), 9);
}

#[test]
fn audio_device_lost_mid_recording_notifies_once_and_keeps_recording() {
    let fake = desktop();
    let mut instance = recording(&fake);
    fake.play_audio(0.5, ms(200));
    record_for(&fake, &mut instance, ms(200));

    fake.fail_audio_reads("音频设备已移除");
    record_for(&fake, &mut instance, ms(300));
    let video = stop_and_read_video(&fake, &mut instance);

    let titles: Vec<_> = fake
        .notifications()
        .iter()
        .map(|n| n.title.clone())
        .collect();
    assert_eq!(titles, ["录屏的声音中断了", "录屏已保存"]);
    assert!(fake.notifications()[0].body.contains("音频设备已移除"));
    assert_eq!(
        video.audio.expect("已录下的声音保留").samples,
        segments(&[(0.5, 200), (0.0, 300)]),
        "之后的部分是静音，画面照常录到停止"
    );
}

#[test]
fn no_audio_device_records_silent_video_and_notifies() {
    let fake = desktop().without_audio_device("找不到音频输出设备");
    let mut instance = recording(&fake);

    let notice = &fake.notifications()[0];
    assert_eq!(notice.title, "录屏没有声音");
    assert!(
        notice.body.contains("找不到音频输出设备"),
        "{}",
        notice.body
    );
    record_for(&fake, &mut instance, ms(300));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("音轨留着").samples,
        segments(&[(0.0, 300)]),
        "照常录制，音轨是静音"
    );
    assert_eq!(video.frames.len(), 9);
    assert_eq!(fake.notifications()[1].title, "录屏已保存");
}
