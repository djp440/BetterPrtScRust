//! 录屏控制条：录屏期间贴在区域边框外，显示已录时长，提供暂停 / 继续和停止。
//!
//! 时间由测试推进：`advance_clock` 让单调时间前进，`tick` 生成节拍事件。录出的视频用
//! `decode::video` 读回；期望的画面来自假平台的 `desktop_pattern`。

use std::time::Duration;

use bps_core::ResidentInstance;
use bps_core::default_bindings;
use bps_platform::{
    Hotkey, Image, Key, Modifiers, OverlayInput, PlatformEvent, Point, PointerButton,
    RecordingToolbarView, Rect,
};
use bps_platform_fake::decode::{self, FakeVideo};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

const ALT_PRINT_SCREEN: Hotkey = Hotkey::new(
    Modifiers {
        ctrl: false,
        alt: true,
        shift: false,
        win: false,
    },
    Key::PrintScreen,
);

/// 一块 640 × 480、缩放 100% 的显示器；鼠标指针在左上角（不在下面用到的区域里）。
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

/// 按 Alt+PrtSc（默认绑定：区域录屏）、框选 `region` 后正在录制的常驻实例。
fn recording(fake: &FakePlatform, region: Rect) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    feed(&mut instance, fake.select_region(region));
    assert!(fake.capturing_video(), "确定区域后应开始录制");
    instance
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

fn toolbar(fake: &FakePlatform) -> RecordingToolbarView {
    fake.recording_toolbar().expect("录制期间应显示录屏控制条")
}

/// 矩形的右边界和下边界（不含）。
fn right(rect: Rect) -> i32 {
    rect.x + rect.width as i32
}

fn bottom(rect: Rect) -> i32 {
    rect.y + rect.height as i32
}

#[test]
fn toolbar_with_timer_pause_and_stop_appears_below_region_when_recording_starts() {
    let fake = desktop();
    let region = Rect::new(100, 100, 200, 150);
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    assert_eq!(fake.recording_toolbar(), None, "框选时还没有控制条");

    feed(&mut instance, fake.select_region(region));

    let view = toolbar(&fake);
    let texts = view.texts();
    for text in ["00:00", "暂停", "停止"] {
        assert!(texts.contains(&text), "控制条上应有“{text}”：{texts:?}");
    }
    let border = view.border.expect("录制范围外应有区域边框");
    assert_eq!(border.rect, region, "边框围住录制范围");
    assert!(border.width >= 1);
    assert!(
        view.bounds.y >= bottom(region) + border.width as i32,
        "控制条在区域边框下方外侧：{:?}",
        view.bounds
    );
    assert!(
        Rect::new(0, 0, 640, 480).intersection(view.bounds) == Some(view.bounds),
        "控制条整个在显示器内：{:?}",
        view.bounds
    );
}

/// 让时间前进 `duration` 后才来一个节拍（事件循环忙、节拍被合并）。
fn tick_after(fake: &FakePlatform, instance: &mut ResidentInstance, duration: Duration) {
    fake.advance_clock(duration);
    instance.handle(fake.tick().unwrap()).unwrap();
}

#[test]
fn timer_shows_whole_seconds_recorded() {
    let fake = desktop();
    let mut instance = recording(&fake, Rect::new(100, 100, 40, 34));

    record_for(&fake, &mut instance, Duration::from_millis(2900));
    assert!(
        toolbar(&fake).texts().contains(&"00:02"),
        "2.9 秒显示 00:02"
    );

    record_for(&fake, &mut instance, Duration::from_millis(100));
    assert!(
        toolbar(&fake).texts().contains(&"00:03"),
        "满 3 秒显示 00:03"
    );

    tick_after(&fake, &mut instance, Duration::from_secs(62));
    assert!(
        toolbar(&fake).texts().contains(&"01:05"),
        "超过一分钟：分:秒"
    );
}

#[test]
fn toolbar_disappears_when_recording_stops() {
    let fake = desktop();
    let mut instance = recording(&fake, Rect::new(100, 100, 200, 150));
    record_for(&fake, &mut instance, Duration::from_millis(300));

    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();

    assert_eq!(fake.recording_toolbar(), None);
    assert_eq!(
        fake.clipboard().files().map(|f| f.len()),
        Some(1),
        "录屏已存储"
    );
}

/// 单色画面。
fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
    Image::from_rgba(width, height, rgba.repeat(width as usize * height as usize))
}

/// 再按一次 Alt+PrtSc 停止录制，读回存下的视频。
fn stop_and_read_video(fake: &FakePlatform, instance: &mut ResidentInstance) -> FakeVideo {
    instance
        .handle(fake.press(ALT_PRINT_SCREEN).unwrap())
        .unwrap();
    let path = fake.clipboard().files().expect("剪贴板应附带录屏文件")[0].clone();
    decode::video(&fake.file(&path).expect("录屏文件应存在"))
}

#[test]
fn pause_stops_timer_and_leaves_paused_screen_out_of_video() {
    let fake = desktop();
    let region = Rect::new(100, 100, 40, 34);
    let mut instance = recording(&fake, region);
    record_for(&fake, &mut instance, Duration::from_secs(1));

    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    let texts = toolbar(&fake).texts().join(" ");
    assert!(texts.contains("继续"), "暂停后按钮变为“继续”：{texts}");
    assert!(!texts.contains("暂停"), "{texts}");
    // 暂停期间画面变了两次，时间过去 3 秒。
    fake.change_display_image(0, solid(640, 480, [9, 99, 199, 255]));
    record_for(&fake, &mut instance, Duration::from_millis(1500));
    fake.change_display_image(0, solid(640, 480, [200, 30, 60, 255]));
    record_for(&fake, &mut instance, Duration::from_millis(1500));
    assert!(toolbar(&fake).texts().contains(&"00:01"), "暂停期间不计时");

    feed(&mut instance, fake.click_recording_toolbar("继续"));
    assert!(toolbar(&fake).texts().contains(&"暂停"));
    record_for(&fake, &mut instance, Duration::from_secs(1));
    assert!(toolbar(&fake).texts().contains(&"00:02"));
    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 60, "只有录制的 2 秒，30 fps");
    let (before, after) = video.frames.split_at(30);
    assert!(
        before.iter().all(|f| f.image == desktop_pattern(region)),
        "暂停前的画面"
    );
    assert!(
        after
            .iter()
            .all(|f| f.image == solid(40, 34, [200, 30, 60, 255])),
        "继续后接着录此刻的画面，暂停期间出现过的画面不在视频里"
    );
    assert_eq!(after[0].timestamp, Duration::from_secs(1), "时间轴连续");
}

#[test]
fn stop_button_saves_recording() {
    let fake = desktop();
    let mut instance = recording(&fake, Rect::new(100, 100, 40, 34));
    record_for(&fake, &mut instance, Duration::from_millis(500));

    feed(&mut instance, fake.click_recording_toolbar("停止"));

    assert_eq!(fake.recording_toolbar(), None);
    assert!(!fake.capturing_video());
    assert_eq!(fake.ticker_period(), None);
    let path = fake.clipboard().files().expect("剪贴板应附带录屏文件")[0].clone();
    assert_eq!(decode::video(&fake.file(&path).unwrap()).frames.len(), 15);
    assert_eq!(fake.notifications()[0].title, "录屏已保存");
}

#[test]
fn stopping_while_paused_saves_only_recorded_time() {
    let fake = desktop();
    let mut instance = recording(&fake, Rect::new(100, 100, 40, 34));
    record_for(&fake, &mut instance, Duration::from_millis(500));
    feed(&mut instance, fake.click_recording_toolbar("暂停"));
    record_for(&fake, &mut instance, Duration::from_secs(2));

    let video = stop_and_read_video(&fake, &mut instance);

    assert_eq!(video.frames.len(), 15, "暂停的 2 秒不算");
}

#[test]
fn button_acts_only_when_released_on_it() {
    let fake = desktop();
    let mut instance = recording(&fake, Rect::new(100, 100, 40, 34));
    let stop = toolbar(&fake).text_rect("停止").unwrap();
    let on_stop = Point::new(stop.x + 1, stop.y + 1);
    let away = Point::new(stop.x - 50, stop.y - 50);

    for input in [
        OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: on_stop,
            modifiers: Modifiers::NONE,
            double_click: false,
        },
        OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: away,
            modifiers: Modifiers::NONE,
        },
    ] {
        instance
            .handle(fake.recording_toolbar_input(input))
            .unwrap();
    }

    assert!(fake.capturing_video(), "按下后移开再松开不算点击");
    assert!(fake.recording_toolbar().is_some());
}

#[test]
fn toolbar_moves_above_region_touching_bottom_of_display() {
    let fake = desktop();
    let region = Rect::new(100, 380, 200, 100);
    let mut instance = recording(&fake, region);
    record_for(&fake, &mut instance, Duration::from_millis(100));

    let view = toolbar(&fake);
    let border = view.border.unwrap().width as i32;
    assert!(
        bottom(view.bounds) <= region.y - border,
        "下方没有空间，控制条换到区域边框上方：{:?}",
        view.bounds
    );
    assert!(Rect::new(0, 0, 640, 480).intersection(view.bounds) == Some(view.bounds));
}
