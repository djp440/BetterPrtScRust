//! 录屏参数（#30）：视频编码、画质 / 码率、麦克风。全局设置给出默认值，每条录屏绑定可以覆盖；
//! 麦克风和系统声音混在同一条音轨里，录制中可以在控制条上分别开关（只影响这一段录屏）。
//!
//! 时间由测试推进：`advance_clock` 让单调时间前进，`tick` 生成节拍事件；`play_audio` / `speak`
//! 让系统声音 / 麦克风从此刻起响一段。假音频源每毫秒一个样本（`FAKE_AUDIO_FORMAT`）。

use std::time::Duration;

use bps_core::{Config, ResidentInstance};
use bps_platform::{
    Color, Hotkey, Key, Modifiers, OverlayItem, PlatformEvent, Point, Rect, VideoCodec,
};
use bps_platform_fake::decode::{self, FakeVideo};
use bps_platform_fake::{FakeDisplay, FakePlatform};

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
/// 足够大、码率不会被下限截住的录制范围。
const LARGE_REGION: Rect = Rect::new(0, 0, 1200, 900);

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 1280, 960)).primary())
        .with_cursor_at(Point::new(1270, 950))
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 用 `globals` 加一条 Alt+PrtSc 区域录屏绑定（`overrides` 是绑定额外的配置行），
/// 框选 `region` 后正在录制的常驻实例。
fn recording_in(
    fake: &FakePlatform,
    globals: &str,
    overrides: &str,
    region: Rect,
) -> ResidentInstance {
    let config = Config::parse(&format!(
        "{globals}\n[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"region\"\nfunction = \"record\"\n{overrides}\n"
    ))
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(region));
    assert!(fake.capturing_video(), "确定区域后应开始录制");
    instance
}

fn recording_with(fake: &FakePlatform, globals: &str, overrides: &str) -> ResidentInstance {
    recording_in(fake, globals, overrides, REGION)
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

/// 录一小段 `LARGE_REGION` 并读出视频。
fn large_video(globals: &str, overrides: &str) -> FakeVideo {
    let fake = desktop();
    let mut instance = recording_in(&fake, globals, overrides, LARGE_REGION);
    record_for(&fake, &mut instance, ms(100));
    stop_and_read_video(&fake, &mut instance)
}

/// 音轨的样本，每段 `(电平, 毫秒数)` 首尾相接。
fn segments(parts: &[(f32, usize)]) -> Vec<f32> {
    parts
        .iter()
        .flat_map(|&(level, count)| std::iter::repeat_n(level, count))
        .collect()
}

fn notification_titles(fake: &FakePlatform) -> Vec<String> {
    fake.notifications()
        .iter()
        .map(|n| n.title.clone())
        .collect()
}

/// 控制条上写着 `label` 的声音开关此刻是否开着（关着的开关文字是半透明的灰色）。
fn switch_on(fake: &FakePlatform, label: &str) -> bool {
    let view = fake.recording_toolbar().expect("录屏控制条没有显示");
    let color: Color = view
        .items
        .iter()
        .find_map(|item| match item {
            OverlayItem::Text { text, color, .. } if text == label => Some(*color),
            _ => None,
        })
        .unwrap_or_else(|| panic!("控制条上没有“{label}”：{:?}", view.texts()));
    color.a == 255
}

// ---- 麦克风 ----

#[test]
fn microphone_is_off_by_default() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "", "");
    assert!(!fake.capturing_microphone());
    fake.speak(0.25, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.audio.unwrap().samples, segments(&[(0.0, 100)]));
}

#[test]
fn microphone_is_mixed_with_system_audio_into_one_track() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "record_microphone = true", "");
    assert!(fake.capturing_audio() && fake.capturing_microphone());
    assert_eq!(fake.opened_microphone(), Some(None), "默认录系统默认麦克风");

    fake.play_audio(0.5, ms(300));
    record_for(&fake, &mut instance, ms(100));
    fake.speak(0.25, ms(200));
    record_for(&fake, &mut instance, ms(300));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.expect("MP4 应带音轨").samples,
        segments(&[(0.5, 100), (0.75, 200), (0.0, 100)]),
        "两路声音按采集时间叠加在同一条音轨里"
    );
    assert!(!fake.capturing_microphone(), "录完停止采集麦克风");
}

#[test]
fn chosen_microphone_is_recorded() {
    let fake = desktop().with_microphones(&[("mic-a", "麦克风 A"), ("mic-b", "麦克风 B")]);
    let mut instance = recording_with(
        &fake,
        "record_microphone = true\nmicrophone = \"mic-b\"",
        "",
    );

    assert_eq!(fake.opened_microphone(), Some(Some("mic-b".to_owned())));
    assert!(fake.notifications().is_empty());
    record_for(&fake, &mut instance, ms(100));
    stop_and_read_video(&fake, &mut instance);
}

#[test]
fn missing_microphone_falls_back_to_default_and_notifies() {
    let fake = desktop();
    let mut instance = recording_with(
        &fake,
        "record_microphone = true\nmicrophone = \"unplugged-mic\"",
        "",
    );

    assert_eq!(fake.opened_microphone(), Some(None), "改用系统默认麦克风");
    let notice = &fake.notifications()[0];
    assert_eq!(notice.title, "录屏改用默认麦克风");
    assert!(notice.body.contains("unplugged-mic"), "{}", notice.body);
    fake.speak(0.25, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);
    assert_eq!(video.audio.unwrap().samples, segments(&[(0.25, 100)]));
}

#[test]
fn no_microphone_records_system_audio_and_notifies() {
    let fake = desktop().with_microphone_failing("没有可用的麦克风");
    let mut instance = recording_with(&fake, "record_microphone = true", "");

    let notice = &fake.notifications()[0];
    assert_eq!(notice.title, "录屏没有麦克风声音");
    assert!(notice.body.contains("没有可用的麦克风"), "{}", notice.body);
    fake.play_audio(0.5, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);
    assert_eq!(
        video.audio.unwrap().samples,
        segments(&[(0.5, 100)]),
        "系统声音照常录制"
    );
}

#[test]
fn microphone_lost_mid_recording_notifies_once() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "record_microphone = true", "");
    fake.speak(0.25, ms(100));
    record_for(&fake, &mut instance, ms(100));

    fake.fail_microphone_reads("麦克风已拔出");
    record_for(&fake, &mut instance, ms(200));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        notification_titles(&fake),
        ["录屏的麦克风中断了", "录屏已保存"]
    );
    assert!(fake.notifications()[0].body.contains("麦克风已拔出"));
    assert_eq!(
        video.audio.unwrap().samples,
        segments(&[(0.25, 100), (0.0, 200)])
    );
}

#[test]
fn binding_overrides_global_microphone() {
    let fake = desktop();
    let mut instance = recording_with(
        &fake,
        "record_microphone = true",
        "record_microphone = false",
    );

    assert!(!fake.capturing_microphone(), "绑定的设置优先");
    record_for(&fake, &mut instance, ms(100));
    stop_and_read_video(&fake, &mut instance);
}

// ---- 控制条上的声音开关 ----

#[test]
fn toolbar_shows_audio_switches_reflecting_settings() {
    let fake = desktop();
    let _instance = recording_with(&fake, "record_microphone = true", "");

    let view = fake.recording_toolbar().unwrap();
    assert_eq!(
        view.texts(),
        ["●", "00:00", "系统声音", "麦克风", "暂停", "停止"]
    );
    assert!(switch_on(&fake, "系统声音"));
    assert!(switch_on(&fake, "麦克风"));
}

#[test]
fn toolbar_microphone_switch_mutes_and_unmutes_mid_recording() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "record_microphone = true", "");
    fake.speak(0.25, ms(100));
    record_for(&fake, &mut instance, ms(100));

    feed(&mut instance, fake.click_recording_toolbar("麦克风"));
    assert!(!fake.capturing_microphone(), "关掉后停止采集麦克风");
    assert!(!switch_on(&fake, "麦克风"));
    fake.speak(0.9, ms(100));
    record_for(&fake, &mut instance, ms(100));

    feed(&mut instance, fake.click_recording_toolbar("麦克风"));
    assert!(fake.capturing_microphone());
    assert!(switch_on(&fake, "麦克风"));
    fake.speak(0.25, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.unwrap().samples,
        segments(&[(0.25, 100), (0.0, 100), (0.25, 100)]),
        "关着的时段是静音"
    );
}

#[test]
fn toolbar_turns_on_system_audio_that_was_off() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "record_system_audio = false", "");
    assert!(!switch_on(&fake, "系统声音"));
    fake.play_audio(0.9, ms(100));
    record_for(&fake, &mut instance, ms(100));

    feed(&mut instance, fake.click_recording_toolbar("系统声音"));
    assert!(fake.capturing_audio());
    fake.play_audio(0.5, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        video.audio.unwrap().samples,
        segments(&[(0.0, 100), (0.5, 100)])
    );
}

#[test]
fn toolbar_switch_affects_only_the_current_recording() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "", "");
    feed(&mut instance, fake.click_recording_toolbar("系统声音"));
    assert!(!fake.capturing_audio());
    record_for(&fake, &mut instance, ms(100));
    stop_and_read_video(&fake, &mut instance);

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(REGION));
    assert!(fake.capturing_audio(), "下一段录屏仍按设置录系统声音");
    assert!(switch_on(&fake, "系统声音"));
}

#[test]
fn toolbar_microphone_switch_failure_notifies_and_stays_off() {
    let fake = desktop().with_microphone_failing("没有可用的麦克风");
    let mut instance = recording_with(&fake, "", "");

    feed(&mut instance, fake.click_recording_toolbar("麦克风"));

    assert_eq!(notification_titles(&fake), ["录屏没有麦克风声音"]);
    assert!(!switch_on(&fake, "麦克风"));
    record_for(&fake, &mut instance, ms(100));
    stop_and_read_video(&fake, &mut instance);
}

#[test]
fn gif_toolbar_has_no_audio_switches() {
    let fake = desktop();
    let _instance = recording_with(&fake, "record_microphone = true", "output_format = \"gif\"");

    assert!(!fake.capturing_microphone(), "GIF 不录声音");
    let view = fake.recording_toolbar().unwrap();
    assert_eq!(view.texts(), ["●", "00:00", "暂停", "停止"]);
}

#[test]
fn audio_encoder_failure_without_requested_audio_is_silent() {
    let fake = desktop().with_audio_encoder_failing("没有可用的 AAC 编码器");
    let mut instance = recording_with(&fake, "record_system_audio = false", "");

    assert!(
        fake.notifications().is_empty(),
        "没要声音时退回无音轨视频不必提示"
    );
    let view = fake.recording_toolbar().unwrap();
    assert!(
        !view.texts().contains(&"系统声音"),
        "没有音轨就没有声音开关"
    );
    record_for(&fake, &mut instance, ms(100));
    assert_eq!(stop_and_read_video(&fake, &mut instance).audio, None);
}

// ---- 视频编码 ----

#[test]
fn mp4_uses_h264_by_default() {
    assert_eq!(large_video("", "").codec, VideoCodec::H264);
}

#[test]
fn chosen_codec_is_used_and_binding_overrides_it() {
    assert_eq!(
        large_video("video_codec = \"h265\"", "").codec,
        VideoCodec::H265
    );
    assert_eq!(
        large_video("video_codec = \"h265\"", "video_codec = \"av1\"").codec,
        VideoCodec::Av1
    );
}

#[test]
fn unsupported_codec_falls_back_to_h264_and_notifies() {
    let fake = desktop().without_video_codec(VideoCodec::Av1, "没有 AV1 编码器");
    let mut instance = recording_with(&fake, "video_codec = \"av1\"", "");

    let notice = &fake.notifications()[0];
    assert_eq!(notice.title, "录屏改用 H.264");
    assert!(
        notice.body.contains("AV1") && notice.body.contains("没有 AV1 编码器"),
        "{}",
        notice.body
    );
    fake.play_audio(0.5, ms(100));
    record_for(&fake, &mut instance, ms(100));
    let video = stop_and_read_video(&fake, &mut instance);
    assert_eq!(video.codec, VideoCodec::H264);
    assert_eq!(
        video.audio.unwrap().samples,
        segments(&[(0.5, 100)]),
        "声音不受影响"
    );
}

// ---- 画质 / 码率 ----

#[test]
fn quality_tiers_raise_the_bitrate() {
    let low = large_video("video_quality = \"low\"", "").bitrate;
    let standard = large_video("", "").bitrate;
    let high = large_video("video_quality = \"high\"", "").bitrate;

    assert!(low < standard && standard < high, "{low} {standard} {high}");
    assert_eq!(
        large_video("video_quality = \"standard\"", "").bitrate,
        standard,
        "默认是标准画质"
    );
}

#[test]
fn efficient_codecs_use_a_lower_bitrate_for_the_same_quality() {
    let h264 = large_video("", "").bitrate;
    let h265 = large_video("video_codec = \"h265\"", "").bitrate;
    let av1 = large_video("video_codec = \"av1\"", "").bitrate;

    assert!(h265 < h264, "{h265} {h264}");
    assert_eq!(av1, h265);
}

#[test]
fn bitrate_grows_with_frame_rate() {
    let at_30 = large_video("", "").bitrate;
    let at_60 = large_video("frame_rate = 60", "");

    assert_eq!(at_60.frame_rate, 60, "全局帧率生效");
    assert_eq!(at_60.bitrate, at_30 * 2);
}

#[test]
fn small_regions_get_the_minimum_bitrate() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "video_quality = \"low\"", "");
    record_for(&fake, &mut instance, ms(100));
    assert_eq!(stop_and_read_video(&fake, &mut instance).bitrate, 500_000);
}

#[test]
fn custom_bitrate_wins_over_quality_and_binding_overrides_global() {
    assert_eq!(
        large_video("video_quality = \"high\"\nvideo_bitrate = 6000", "").bitrate,
        6_000_000
    );
    assert_eq!(
        large_video("video_bitrate = 6000", "video_bitrate = 2500").bitrate,
        2_500_000
    );
    assert!(
        large_video("video_bitrate = 6000", "video_quality = \"low\"").bitrate < 6_000_000,
        "绑定写了画质档位时用档位"
    );
}

#[test]
fn binding_frame_rate_overrides_global_for_gif_too() {
    let fake = desktop();
    let mut instance = recording_with(&fake, "frame_rate = 20", "output_format = \"gif\"");
    record_for(&fake, &mut instance, ms(100));
    assert_eq!(stop_and_read_video(&fake, &mut instance).frame_rate, 20);

    let fake = desktop();
    let mut instance = recording_with(
        &fake,
        "frame_rate = 20",
        "output_format = \"gif\"\nframe_rate = 10",
    );
    record_for(&fake, &mut instance, ms(100));
    assert_eq!(stop_and_read_video(&fake, &mut instance).frame_rate, 10);
}
