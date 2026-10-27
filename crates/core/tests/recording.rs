//! 录屏（区域）：在实时画面上框选区域后开始录制，再按一次同一个热键停止；录完按热键绑定的
//! 配置自动存储，剪贴板里附带这个文件。
//!
//! 时间由测试推进：`advance_clock` 让单调时间前进，`tick` 生成节拍事件。录出的视频用
//! `decode::video` 读回；期望的画面来自假平台的 `desktop_pattern`（每个虚拟桌面像素的颜色由坐标决定）。

use std::path::PathBuf;
use std::time::Duration;

use bps_core::{
    CliStatus, Config, CoreError, Flow, Locale, ResidentInstance, default_bindings, run_cli,
};
use bps_platform::{
    Backdrop, Hotkey, Image, Key, LocalDateTime, Modifiers, NotificationClick, OverlayKey,
    PlatformEvent, Point, Rect, VideoFormat,
};
use bps_platform_fake::decode::{self, FakeVideo};
use bps_platform_fake::{
    FakeDisplay, FakePlatform, FakeResidentChannel, desktop_pattern, draw_fake_cursor,
};

const ALT_PRINT_SCREEN: Hotkey = Hotkey::new(
    Modifiers {
        ctrl: false,
        alt: true,
        shift: false,
        win: false,
    },
    Key::PrintScreen,
);

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 128, 96)).primary())
        .with_cursor_at(Point::new(30, 20))
}

/// 使用默认热键绑定、已注册热键的常驻实例。
fn resident(fake: &FakePlatform) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

#[test]
fn alt_print_screen_frames_region_on_live_screen() {
    let fake = single_display();
    let mut instance = resident(&fake);

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    assert_eq!(
        fake.overlay_backdrop(),
        Some(Backdrop::Live),
        "录屏应在实时画面上框选，而不是冻结帧"
    );
    assert!(!fake.capturing_video(), "框选完成前不应开始录制");
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

/// 默认存储目录里的录屏文件。
fn saved_recording(fake: &FakePlatform, name: &str) -> PathBuf {
    fake.pictures_dir().join("BetterPrtSc").join(name)
}

#[test]
fn recording_runs_until_same_hotkey_pressed_again_and_is_saved_and_copied() {
    let fake = single_display();
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    // 鼠标指针 (30, 20) 在区域内。
    let region = Rect::new(20, 10, 40, 36);
    feed(&mut instance, fake.select_region(region));
    assert!(!fake.overlay_visible(), "开始录制后遮罩应关闭");
    assert!(fake.capturing_video(), "确定区域后应开始录制");

    record_for(&fake, &mut instance, Duration::from_secs(1));
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    assert!(!fake.capturing_video(), "停止后不再捕获画面");
    assert_eq!(fake.ticker_period(), None, "停止后节拍应停止");
    let path = saved_recording(&fake, "2000-01-01 00-00-00.mp4");
    assert_eq!(
        fake.files(),
        std::slice::from_ref(&path),
        "只留下录好的 MP4，没有临时文件"
    );
    let video = decode::video(&fake.file(&path).unwrap());
    assert_eq!(video.format, VideoFormat::Mp4);
    assert_eq!((video.width, video.height), (40, 36));
    assert_eq!(video.frame_rate, 30, "MP4 默认 30 fps");
    assert_eq!(video.frames.len(), 30, "录了 1 秒，30 fps");
    let timestamps: Vec<Duration> = video.frames.iter().map(|f| f.timestamp).collect();
    assert_eq!(timestamps[0], Duration::ZERO);
    assert_eq!(timestamps[1], Duration::from_nanos(33_333_333));
    assert_eq!(timestamps[3], Duration::from_millis(100));
    assert_eq!(timestamps[29], Duration::from_nanos(966_666_666));
    let expected = draw_fake_cursor(
        &desktop_pattern(region),
        Point::new(region.x, region.y),
        Point::new(30, 20),
    );
    assert!(
        video.frames.iter().all(|f| f.image == expected),
        "每一帧都是区域内的实时画面，默认带鼠标指针"
    );

    assert_eq!(fake.clipboard().formats(), ["Files"], "录屏只附带文件");
    assert_eq!(fake.clipboard().files(), Some(vec![path.clone()]));
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "录屏已保存");
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenFile(path))
    );
}

/// 按 Alt+PrtSc、框选 `region` 后正在录制的常驻实例。
fn recording(fake: &FakePlatform, region: Rect) -> ResidentInstance {
    let mut instance = resident(fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(region));
    assert!(fake.capturing_video(), "确定区域后应开始录制");
    instance
}

/// 再按一次 Alt+PrtSc 停止录制，读回存下的视频。
fn stop_and_read_video(fake: &FakePlatform, instance: &mut ResidentInstance) -> FakeVideo {
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    let path = fake.clipboard().files().expect("剪贴板应附带录屏文件")[0].clone();
    decode::video(&fake.file(&path).expect("录屏文件应存在"))
}

/// 单色画面。
fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
    Image::from_rgba(width, height, rgba.repeat(width as usize * height as usize))
}

#[test]
fn recording_follows_live_screen_changes() {
    let fake = single_display().with_cursor_at(Point::new(0, 0));
    let region = Rect::new(8, 8, 40, 34);
    let mut instance = recording(&fake, region);

    record_for(&fake, &mut instance, Duration::from_millis(500));
    fake.change_display_image(0, solid(128, 96, [9, 99, 199, 255]));
    record_for(&fake, &mut instance, Duration::from_millis(500));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 30);
    let (before, after): (Vec<_>, Vec<_>) = video
        .frames
        .iter()
        .partition(|f| f.timestamp < Duration::from_millis(500));
    assert!(before.iter().all(|f| f.image == desktop_pattern(region)));
    assert!(
        after
            .iter()
            .all(|f| f.image == solid(40, 34, [9, 99, 199, 255])),
        "画面变化后录到的是新画面"
    );
}

/// 只有一条 Alt+PrtSc 录屏绑定的常驻实例；`overrides` 是这条绑定额外的配置行。
fn resident_with_record_binding(fake: &FakePlatform, overrides: &str) -> ResidentInstance {
    let config = Config::parse(&format!(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"region\"\nfunction = \"record\"\n{overrides}\n"
    ))
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
}

#[test]
fn binding_sets_frame_rate_and_hides_cursor() {
    let fake = single_display();
    let mut instance = resident_with_record_binding(
        &fake,
        "output_format = \"mp4\"\nframe_rate = 10\nshow_cursor = false",
    );
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    // 鼠标指针 (30, 20) 在区域内，但不应被录进去。
    let region = Rect::new(20, 10, 40, 36);
    feed(&mut instance, fake.select_region(region));

    record_for(&fake, &mut instance, Duration::from_secs(1));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frame_rate, 10);
    let timestamps: Vec<u64> = video
        .frames
        .iter()
        .map(|f| f.timestamp.as_millis() as u64)
        .collect();
    assert_eq!(timestamps, [0, 100, 200, 300, 400, 500, 600, 700, 800, 900]);
    assert!(
        video
            .frames
            .iter()
            .all(|f| f.image == desktop_pattern(region)),
        "关闭显示鼠标指针后画面里没有指针"
    );
}

#[test]
fn late_and_merged_ticks_keep_constant_frame_rate() {
    let fake = single_display();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));

    // 事件循环忙了 0.7 秒才处理到一个节拍，之后节拍又挤在一起到达。
    fake.advance_clock(Duration::from_millis(700));
    instance.handle(fake.tick().unwrap()).unwrap();
    for _ in 0..3 {
        instance.handle(fake.tick().unwrap()).unwrap();
    }
    fake.advance_clock(Duration::from_millis(300));
    instance.handle(fake.tick().unwrap()).unwrap();
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 30, "1 秒仍是 30 帧");
    assert_eq!(video.duration(), Duration::from_nanos(999_999_999));
}

#[test]
fn odd_sized_region_is_trimmed_to_even_size_for_mp4() {
    let fake = single_display().with_cursor_at(Point::new(0, 0));
    let mut instance = recording(&fake, Rect::new(5, 7, 41, 35));

    record_for(&fake, &mut instance, Duration::from_millis(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        (video.width, video.height),
        (40, 34),
        "H.264 的宽高必须是偶数"
    );
    assert!(
        video
            .frames
            .iter()
            .all(|f| f.image == desktop_pattern(Rect::new(5, 7, 40, 34))),
        "去掉的是最右一列和最下一行"
    );
}

#[test]
fn stopping_right_after_start_still_saves_one_frame() {
    let fake = single_display();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));

    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 1);
}

/// 剪贴板、文件、通知都没有任何产物，也没有在捕获画面。
fn assert_no_product(fake: &FakePlatform) {
    assert!(fake.clipboard().is_empty(), "剪贴板不应有内容");
    assert!(fake.files().is_empty(), "不应留下文件：{:?}", fake.files());
    assert!(!fake.capturing_video(), "不应还在捕获画面");
    assert_eq!(fake.ticker_period(), None, "节拍应已停止");
}

#[test]
fn escape_while_framing_cancels_without_recording() {
    let fake = single_display();
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    feed(
        &mut instance,
        fake.drag(Point::new(5, 5), Point::new(20, 20)),
    );
    instance.handle(fake.press_key(OverlayKey::Escape)).unwrap();

    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
    assert!(fake.notifications().is_empty());
}

#[test]
fn nothing_is_saved_when_no_frame_arrived_before_stop() {
    let fake = single_display().with_video_frames_delayed();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));
    fake.advance_clock(Duration::from_millis(200));
    instance.handle(fake.tick().unwrap()).unwrap();

    let result = instance.handle(fake.press(ALT_PRINT_SCREEN).unwrap());

    assert_eq!(result, Err(CoreError::NothingRecorded));
    assert_no_product(&fake);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert!(notifications[0].body.contains("没有录到任何画面"));
}

#[test]
fn frames_arriving_late_fill_the_start_of_the_recording() {
    let fake = single_display()
        .with_cursor_at(Point::new(60, 40))
        .with_video_frames_delayed();
    let region = Rect::new(0, 0, 40, 40);
    let mut instance = recording(&fake, region);
    fake.advance_clock(Duration::from_millis(100));
    instance.handle(fake.tick().unwrap()).unwrap();

    fake.deliver_video_frames();
    record_for(&fake, &mut instance, Duration::from_millis(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 6, "0.2 秒，30 fps");
    assert!(
        video
            .frames
            .iter()
            .all(|f| f.image == desktop_pattern(region))
    );
}

const SHIFT_PRINT_SCREEN: Hotkey = Hotkey::new(
    Modifiers {
        ctrl: false,
        alt: false,
        shift: true,
        win: false,
    },
    Key::PrintScreen,
);

#[test]
fn other_capture_hotkeys_are_refused_while_recording() {
    let fake = single_display();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));
    record_for(&fake, &mut instance, Duration::from_millis(100));

    let result = instance.handle(fake.press(SHIFT_PRINT_SCREEN).unwrap());

    assert!(
        matches!(result, Err(CoreError::RecordingInProgress { .. })),
        "{result:?}"
    );
    assert!(
        fake.files()
            .iter()
            .all(|f| f.extension() == Some("part".as_ref())),
        "没有截图"
    );
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "正在录屏");
    assert!(
        notifications[0].body.contains("Alt+PrintScreen"),
        "提示用哪个热键停止：{}",
        notifications[0].body
    );

    assert!(fake.capturing_video(), "录制不受影响");
    record_for(&fake, &mut instance, Duration::from_millis(100));
    let video = stop_and_read_video(&fake, &mut instance);
    assert_eq!(video.frames.len(), 6);
}

#[test]
fn pressing_record_hotkey_again_while_framing_keeps_the_overlay() {
    let fake = single_display();
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    let result = instance.handle(fake.press(ALT_PRINT_SCREEN).unwrap());

    assert_eq!(result, Err(CoreError::CaptureInProgress));
    assert_eq!(fake.overlay_backdrop(), Some(Backdrop::Live), "遮罩还在");
    assert!(fake.notifications().is_empty(), "遮罩看得见，不另发通知");
}

#[test]
fn exiting_while_recording_saves_what_was_recorded() {
    let fake = single_display();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));
    record_for(&fake, &mut instance, Duration::from_millis(200));

    let flow = instance.handle(PlatformEvent::ExitRequested).unwrap();

    assert_eq!(flow, Flow::Exit);
    assert!(!fake.capturing_video());
    let path = saved_recording(&fake, "2000-01-01 00-00-00.mp4");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    assert_eq!(decode::video(&fake.file(&path).unwrap()).frames.len(), 6);
}

#[test]
fn recording_stops_and_is_saved_when_its_target_disappears() {
    let fake = single_display();
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));
    record_for(&fake, &mut instance, Duration::from_millis(200));

    // 例如显示器被拔掉。
    fake.end_video_source();
    fake.advance_clock(Duration::from_millis(100));
    instance.handle(fake.tick().unwrap()).unwrap();

    assert!(!fake.capturing_video(), "目标消失后停止录制");
    assert_eq!(fake.ticker_period(), None);
    let path = saved_recording(&fake, "2000-01-01 00-00-00.mp4");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    assert_eq!(fake.clipboard().files(), Some(vec![path.clone()]));
    let video = decode::video(&fake.file(&path).unwrap());
    assert_eq!(video.frames.len(), 9, "保留到停止那一刻（0.3 秒）的内容");
    assert_eq!(fake.notifications()[0].title, "录屏已保存");
}

#[test]
fn encoder_failure_is_reported_and_leaves_nothing_behind() {
    let fake = single_display().with_video_encoder_failing("没有可用的 H.264 编码器");
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    let mut events = fake.select_region(Rect::new(0, 0, 40, 40));
    let confirm = events.pop().unwrap();
    feed(&mut instance, events);
    let result = instance.handle(confirm);

    assert!(matches!(result, Err(CoreError::Platform(_))), "{result:?}");
    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert!(notifications[0].body.contains("没有可用的 H.264 编码器"));
}

/// 输出格式为 GIF 的录屏绑定：框选 `region` 后录 `duration` 再停止，读回存下的视频。
fn record_gif(fake: &FakePlatform, overrides: &str, region: Rect, duration: Duration) -> FakeVideo {
    let mut instance = resident_with_record_binding(
        fake,
        &format!(
            "output_format = \"gif\"
{overrides}"
        ),
    );
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(region));
    assert!(fake.capturing_video(), "确定区域后应开始录制");
    record_for(fake, &mut instance, duration);
    stop_and_read_video(fake, &mut instance)
}

#[test]
fn gif_binding_records_gif_file_at_15_fps_by_default() {
    let fake = single_display();
    // 奇数宽高：GIF 没有 MP4 的偶数限制，不裁剪。
    let region = Rect::new(20, 10, 41, 35);

    let video = record_gif(&fake, "", region, Duration::from_secs(1));

    let path = saved_recording(&fake, "2000-01-01 00-00-00.gif");
    assert_eq!(fake.files(), [path], "只留下录好的 GIF，没有临时文件");
    assert_eq!(video.format, VideoFormat::Gif);
    assert_eq!((video.width, video.height), (41, 35));
    assert_eq!(video.frame_rate, 15, "GIF 默认 15 fps");
    assert_eq!(video.frames.len(), 15);
    assert_eq!(video.frames[3].timestamp, Duration::from_millis(200));
    let expected = draw_fake_cursor(
        &desktop_pattern(region),
        Point::new(region.x, region.y),
        Point::new(30, 20),
    );
    assert!(video.frames.iter().all(|f| f.image == expected));
    assert_eq!(fake.notifications()[0].title, "录屏已保存");
}

#[test]
fn gif_accepts_regions_smaller_than_mp4_minimum() {
    let fake = single_display();

    let video = record_gif(
        &fake,
        "",
        Rect::new(20, 10, 10, 8),
        Duration::from_millis(400),
    );

    assert_eq!((video.width, video.height), (10, 8));
    assert_eq!(video.frames.len(), 6);
}

#[test]
fn frame_rate_binding_applies_to_gif() {
    let fake = single_display();

    let video = record_gif(
        &fake,
        "frame_rate = 5",
        Rect::new(20, 10, 40, 36),
        Duration::from_secs(1),
    );

    assert_eq!(video.format, VideoFormat::Gif);
    assert_eq!(video.frame_rate, 5);
    assert_eq!(video.frames.len(), 5);
    assert_eq!(video.frames[1].timestamp, Duration::from_millis(200));
}

#[test]
fn existing_file_with_same_name_is_not_overwritten() {
    let existing = saved_recording(&FakePlatform::new(), "2000-01-01 00-00-00.mp4");
    let fake = single_display().with_file(&existing, b"old".to_vec());
    let mut instance = recording(&fake, Rect::new(0, 0, 40, 40));
    record_for(&fake, &mut instance, Duration::from_millis(100));

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    assert_eq!(fake.file(&existing).unwrap(), b"old", "已有文件保持原样");
    let path = saved_recording(&fake, "2000-01-01 00-00-00-1.mp4");
    assert_eq!(fake.files(), [path.clone(), existing]);
    assert_eq!(fake.clipboard().files(), Some(vec![path]));
}

#[test]
fn recording_is_saved_to_binding_save_dir_with_its_template() {
    let fake = single_display().with_time(LocalDateTime::new(2026, 10, 24, 18, 5, 9));
    let mut instance = resident_with_record_binding(
        &fake,
        r#"save_dir = 'D:\Videos'
filename_template = "录屏 {HH-mm-ss} {mode}""#,
    );
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(Rect::new(0, 0, 40, 40)));
    record_for(&fake, &mut instance, Duration::from_millis(100));

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    let path = PathBuf::from(r"D:\Videos\录屏 18-05-09 region.mp4");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    assert_eq!(fake.clipboard().files(), Some(vec![path]));
}

#[test]
fn unwritable_save_dir_fails_before_recording_starts() {
    let fake = single_display().with_read_only_dir(r"D:\ReadOnly");
    let mut instance = resident_with_record_binding(&fake, r"save_dir = 'D:\ReadOnly\Videos'");
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    let mut events = fake.select_region(Rect::new(0, 0, 40, 40));
    let confirm = events.pop().unwrap();
    feed(&mut instance, events);
    let result = instance.handle(confirm);

    assert!(matches!(result, Err(CoreError::Platform(_))), "{result:?}");
    assert!(!fake.capturing_video());
    assert_eq!(fake.ticker_period(), None);
    assert!(fake.clipboard().is_empty());
    assert_eq!(fake.notifications().len(), 1);
}

#[test]
fn bps_capture_region_with_record_binding_replies_after_recording_is_saved() {
    let fake = single_display();
    let mut instance = resident_with_record_binding(&fake, "");
    let user = fake.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        instance.handle(event).unwrap();
        // 坐在屏幕前的用户：框选区域，录 0.2 秒，再按一次 Alt+PrtSc。
        feed(&mut instance, user.select_region(Rect::new(0, 0, 40, 40)));
        record_for(&user, &mut instance, Duration::from_millis(200));
        instance
            .handle(user.press(ALT_PRINT_SCREEN).unwrap())
            .unwrap();
    });

    let outcome = run_cli(["capture", "region"], &mut channel, Locale::ZhCn);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    let path = saved_recording(&fake, "2000-01-01 00-00-00.mp4");
    assert_eq!(fake.clipboard().files(), Some(vec![path]));
}

#[test]
fn display_mode_records_display_under_cursor_without_framing() {
    let second = Rect::new(128, -10, 40, 34);
    let fake = single_display()
        .with_display(FakeDisplay::new(second))
        .with_cursor_at(Point::new(200, 200));
    let config = Config::parse(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"display\"\nfunction = \"record\"\n",
    )
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    fake.move_cursor(Point::new(130, 0));

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    assert!(!fake.overlay_visible(), "显示器模式不需要框选");
    fake.move_cursor(Point::new(200, 200));
    record_for(&fake, &mut instance, Duration::from_millis(100));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!((video.width, video.height), (40, 34));
    assert!(
        video
            .frames
            .iter()
            .all(|f| f.image == desktop_pattern(second))
    );
}

#[test]
fn virtual_desktop_mode_records_all_displays() {
    let fake = single_display()
        .with_display(FakeDisplay::new(Rect::new(128, -10, 40, 34)))
        .with_cursor_at(Point::new(500, 500));
    let config = Config::parse(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"virtual_desktop\"\nfunction = \"record\"\n",
    )
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    record_for(&fake, &mut instance, Duration::from_millis(100));
    let video = stop_and_read_video(&fake, &mut instance);

    // 虚拟桌面 (0, -10)–(168, 96)：168 × 106。
    assert_eq!((video.width, video.height), (168, 106));
    let frame = &video.frames[0].image;
    assert_eq!(
        frame.pixel(130, 5),
        desktop_pattern(Rect::new(130, -5, 1, 1)).pixel(0, 0)
    );
    assert_eq!(frame.pixel(0, 0), [0, 0, 0, 0], "显示器之间的空隙是透明的");
}

#[test]
fn region_smaller_than_mp4_minimum_is_refused() {
    let fake = single_display();
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    // 去掉最下一行后高 32：Windows 上的 H.264 编码器要求宽和高都大于 32 像素。
    let region = Rect::new(10, 10, 40, 33);
    let mut results: Vec<_> = fake
        .select_region(region)
        .into_iter()
        .map(|event| instance.handle(event))
        .collect();

    assert_eq!(
        results.pop().unwrap(),
        Err(CoreError::RecordingRegionTooSmall {
            region,
            minimum: 34
        })
    );
    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(
        notifications[0].body,
        "区域 40 × 33 太小，无法录屏（宽和高至少 34 像素）"
    );
}
