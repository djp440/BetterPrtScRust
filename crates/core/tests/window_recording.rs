//! 窗口录屏：在实时画面上选中一个顶层窗口后持续录制它；录的是窗口自己的内容（被遮挡的部分
//! 也完整），窗口关闭时自动停止并保存已录下的内容，窗口改变大小时画布尺寸不变。
//!
//! 窗口由假平台描述（`FakeWindow`，后添加的在上层）；窗口内容是 `window_pattern`，与桌面画面
//! `desktop_pattern` 不同，所以能区分录到的是窗口还是屏幕上的画面。

use std::time::Duration;

use bps_core::{Config, CoreError, ResidentInstance};
use bps_platform::{Backdrop, Hotkey, Key, Modifiers, PlatformEvent, Point, PointerButton, Rect};
use bps_platform_fake::decode::{self, FakeVideo};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeWindow, window_pattern};

const ALT_PRINT_SCREEN: Hotkey = Hotkey::new(
    Modifiers {
        ctrl: false,
        alt: true,
        shift: false,
        win: false,
    },
    Key::PrintScreen,
);

const TARGET: Rect = Rect::new(10, 10, 80, 60);

/// 桌面上有两个窗口：下层的目标窗口，和上层盖住它右半边的遮挡窗口。
fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 200, 150)).primary())
        .with_cursor_at(Point::new(190, 140))
        .with_window(FakeWindow::new("目标", TARGET))
        .with_window(FakeWindow::new("遮挡", Rect::new(50, 0, 100, 100)))
}

/// 只有一条 Alt+PrtSc“窗口录屏”绑定的常驻实例。
fn resident(fake: &FakePlatform) -> ResidentInstance {
    let config = Config::parse(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"window\"\nactions = [\"record\"]\n",
    )
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 按热键、在目标窗口露出的部分单击后正在录制的常驻实例。
fn recording(fake: &FakePlatform) -> ResidentInstance {
    let mut instance = resident(fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    assert_eq!(fake.overlay_backdrop(), Some(Backdrop::Live));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(20, 20)),
    );
    assert!(!fake.overlay_visible(), "选中窗口后遮罩应关闭");
    assert!(fake.capturing_video(), "选中窗口后应开始录制");
    instance
}

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

fn stop_and_read_video(fake: &FakePlatform, instance: &mut ResidentInstance) -> FakeVideo {
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    read_saved_video(fake)
}

fn read_saved_video(fake: &FakePlatform) -> FakeVideo {
    let path = fake.clipboard().files().expect("剪贴板应附带录屏文件")[0].clone();
    decode::video(&fake.file(&path).expect("录屏文件应存在"))
}

#[test]
fn occluded_window_is_recorded_complete() {
    let fake = desktop();
    let mut instance = recording(&fake);

    record_for(&fake, &mut instance, Duration::from_millis(500));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!((video.width, video.height), (80, 60));
    assert_eq!(video.frames.len(), 15);
    let expected = window_pattern(80, 60);
    assert!(
        video.frames.iter().all(|f| f.image == expected),
        "被遮挡的右半边也是窗口自己的内容，不是屏幕上的遮挡窗口"
    );
}

#[test]
fn closing_window_stops_recording_and_saves_what_was_recorded() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, Duration::from_millis(500));

    fake.close_window(0);
    fake.advance_clock(Duration::from_millis(100));
    instance.handle(fake.tick().unwrap()).unwrap();

    assert!(!fake.capturing_video(), "窗口关闭后应停止录制");
    assert_eq!(fake.ticker_period(), None);
    let video = read_saved_video(&fake);
    assert_eq!((video.width, video.height), (80, 60));
    assert!(
        (15..=18).contains(&video.frames.len()),
        "窗口关闭前录下的内容都保存了：{} 帧",
        video.frames.len()
    );
    assert!(
        video
            .frames
            .iter()
            .all(|f| f.image == window_pattern(80, 60))
    );
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "录屏已保存");
    // 之后可以立即开始下一次录屏。
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    assert!(fake.overlay_visible());
}

#[test]
fn resizing_window_keeps_video_size_and_crops_or_pads_with_black() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, Duration::from_millis(300));

    fake.resize_window(0, 100, 40); // 变宽、变矮
    record_for(&fake, &mut instance, Duration::from_millis(300));
    fake.resize_window(0, 50, 30); // 比开始时小
    record_for(&fake, &mut instance, Duration::from_millis(300));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(
        (video.width, video.height),
        (80, 60),
        "视频尺寸在开始录制时定下，窗口改变大小不影响"
    );
    assert_eq!(video.frames.len(), 27);
    let first = &video.frames[0].image;
    assert_eq!(first, &window_pattern(80, 60));
    let wider = &video.frames[12].image;
    let pixel = |image: &bps_platform::Image, x: u32, y: u32| {
        let i = (y as usize * image.width() as usize + x as usize) * 4;
        image.rgba()[i..i + 4].to_vec()
    };
    let content = window_pattern(100, 40);
    assert_eq!(
        pixel(wider, 79, 39),
        pixel(&content, 79, 39),
        "超出画布的宽度被裁掉"
    );
    assert_eq!(pixel(wider, 0, 0), pixel(&content, 0, 0), "内容贴在左上角");
    assert_eq!(pixel(wider, 5, 50), [0, 0, 0, 255], "窗口变矮后下方补黑");
    let smaller = &video.frames[26].image;
    assert_eq!(
        pixel(smaller, 49, 29),
        pixel(&window_pattern(50, 30), 49, 29)
    );
    assert_eq!(pixel(smaller, 50, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(smaller, 0, 30), [0, 0, 0, 255]);
}

#[test]
fn file_name_uses_window_title() {
    let config = Config::parse(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"window\"\nactions = [\"record\"]\nfilename_template = \"{window_title}\"\n",
    )
    .unwrap();
    let fake = desktop();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(20, 20)),
    );
    record_for(&fake, &mut instance, Duration::from_millis(100));
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    let path = fake.clipboard().files().unwrap()[0].clone();
    assert_eq!(path.file_name().unwrap(), "目标.mp4");
}

#[test]
fn escape_cancels_window_selection_without_recording() {
    let fake = desktop();
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    assert!(fake.overlay_visible());
    instance
        .handle(fake.press_key(bps_platform::OverlayKey::Escape))
        .unwrap();
    assert!(!fake.overlay_visible());
    assert!(!fake.capturing_video());
    assert!(fake.files().is_empty());
}

#[test]
fn window_too_small_for_mp4_is_rejected_without_leaving_files() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 200, 150)).primary())
        .with_cursor_at(Point::new(190, 140))
        .with_window(FakeWindow::new("小", Rect::new(10, 10, 30, 30)));
    let mut instance = resident(&fake);
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    let results: Vec<_> = fake
        .click(PointerButton::Left, Point::new(20, 20))
        .into_iter()
        .map(|event| instance.handle(event))
        .collect();
    assert!(
        matches!(
            results.last(),
            Some(Err(CoreError::RecordingRegionTooSmall { minimum: 34, .. }))
        ),
        "{results:?}"
    );
    assert!(!fake.overlay_visible());
    assert!(!fake.capturing_video());
    assert!(fake.files().is_empty());
}

#[test]
fn window_recording_supports_gif_output() {
    let fake = desktop();
    let config = Config::parse(
        "[[bindings]]\nhotkey = \"Alt+PrintScreen\"\nmode = \"window\"\nactions = [\"record\"]\noutput_format = \"gif\"\n",
    )
    .unwrap();
    let mut instance = ResidentInstance::new(fake.platform(), config.bindings);
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(20, 20)),
    );
    assert!(fake.capturing_video());

    record_for(&fake, &mut instance, Duration::from_secs(1));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.format, bps_platform::VideoFormat::Gif);
    assert_eq!((video.width, video.height), (80, 60));
    assert_eq!(video.frames.len(), 15);
}

#[test]
fn toolbar_appears_while_recording_a_window_and_disappears_when_it_stops() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, Duration::from_secs(1));

    let toolbar = fake.recording_toolbar().expect("录制窗口时应显示控制条");
    let texts = toolbar.texts();
    assert!(
        texts.contains(&"00:01") && texts.contains(&"暂停") && texts.contains(&"停止"),
        "{texts:?}"
    );

    feed(&mut instance, fake.click_recording_toolbar("停止"));

    assert_eq!(fake.recording_toolbar(), None);
    assert!(!fake.capturing_video());
    assert_eq!(read_saved_video(&fake).frames.len(), 30);
}

#[test]
fn pausing_window_recording_stops_timer_and_leaves_paused_time_out_of_video() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, Duration::from_secs(1));

    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    record_for(&fake, &mut instance, Duration::from_secs(3));
    let texts = fake.recording_toolbar().unwrap().texts().join(" ");
    assert!(texts.contains("继续") && texts.contains("00:01"), "{texts}");

    feed(&mut instance, fake.click_recording_toolbar("继续"));
    record_for(&fake, &mut instance, Duration::from_secs(1));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 60, "只有录制的 2 秒，30 fps");
    let expected = window_pattern(80, 60);
    assert!(video.frames.iter().all(|f| f.image == expected));
    assert_eq!(
        video.frames[30].timestamp,
        Duration::from_secs(1),
        "时间轴连续"
    );
}

#[test]
fn window_closing_while_paused_saves_only_recorded_time() {
    let fake = desktop();
    let mut instance = recording(&fake);
    record_for(&fake, &mut instance, Duration::from_millis(500));
    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    record_for(&fake, &mut instance, Duration::from_secs(2));

    fake.close_window(0);
    fake.advance_clock(Duration::from_millis(100));
    instance.handle(fake.tick().unwrap()).unwrap();

    assert!(!fake.capturing_video());
    assert_eq!(fake.recording_toolbar(), None);
    assert_eq!(read_saved_video(&fake).frames.len(), 15, "暂停的时间不算");
}
