//! OCR 直达功能：截图交给结果面板，由结果面板在本机识别文字。
//!
//! 常驻实例只负责打开结果面板并把截图交给它（`StandaloneWindow::ResultPanel`），
//! 识别本身在结果面板里进行，见 `result_panel.rs`。

use bps_core::{CaptureMode, CaptureRequest, DirectFunction, HotkeyBinding, ResidentInstance};
use bps_platform::{Hotkey, PlatformEvent, Point, Rect, StandaloneWindow};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
        .with_cursor_at(Point::new(30, 20))
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn hotkey(text: &str) -> Hotkey {
    text.parse().unwrap()
}

#[test]
fn default_ocr_hotkey_opens_result_panel_for_selected_region() {
    let fake = single_display();
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .handle(fake.press(hotkey("Win+Shift+O")).unwrap())
        .unwrap();
    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));

    assert!(!fake.overlay_visible());
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region),
            translate: false,
        }]
    );
    assert!(fake.clipboard().is_empty(), "只做 OCR 时不应写剪贴板");
    assert!(
        fake.update_check_requests().is_empty(),
        "OCR 不发出任何网络请求"
    );
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn releasing_the_drag_starts_ocr_without_confirming() {
    let fake = single_display();
    let mut instance = instance_with_ocr_binding(&fake);

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(10, 5, 20, 12);
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    assert!(!fake.overlay_visible(), "松开即开始 OCR，不等 Enter");
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region),
            translate: false,
        }]
    );
}

/// 只有一条热键绑定（Ctrl+F9 → 区域 OCR）的常驻实例。
fn instance_with_ocr_binding(fake: &FakePlatform) -> ResidentInstance {
    let binding = HotkeyBinding {
        hotkey: hotkey("Ctrl+F9"),
        capture: CaptureRequest::function(CaptureMode::Region, DirectFunction::Ocr),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    let _ = instance.register_hotkeys();
    instance
}

#[test]
fn result_panel_that_cannot_be_opened_is_reported() {
    let fake = single_display().with_window_launch_failing();
    let mut instance = instance_with_ocr_binding(&fake);

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let mut events = fake.drag(Point::new(3, 4), Point::new(32, 12));
    let release = events.pop().unwrap();
    feed(&mut instance, events);
    let result = instance.handle(release);

    assert!(result.is_err());
    assert!(fake.clipboard().is_empty(), "OCR 不复制");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "无法打开结果面板");
    assert!(
        notifications[0].body.contains("找不到程序文件"),
        "{notifications:?}"
    );
}
