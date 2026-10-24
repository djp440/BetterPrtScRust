//! OCR 截后动作：截图交给结果面板，由结果面板在本机识别文字。
//!
//! 常驻实例只负责打开结果面板并把截图交给它（`StandaloneWindow::ResultPanel`），
//! 识别本身在结果面板里进行，见 `result_panel.rs`。

use bps_core::{CaptureMode, CaptureRequest, HotkeyBinding, PostCaptureAction, ResidentInstance};
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
            screenshot: desktop_pattern(region)
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

/// 只有一条热键绑定（Ctrl+F9 → 区域 + `actions`）的常驻实例。
fn instance_with_region_binding(
    fake: &FakePlatform,
    actions: Vec<PostCaptureAction>,
) -> ResidentInstance {
    let binding = HotkeyBinding {
        hotkey: hotkey("Ctrl+F9"),
        capture: CaptureRequest::new(CaptureMode::Region, actions),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    let _ = instance.register_hotkeys();
    instance
}

#[test]
fn ocr_together_with_copy_does_both() {
    let fake = single_display();
    let mut instance = instance_with_region_binding(
        &fake,
        vec![PostCaptureAction::CopyToClipboard, PostCaptureAction::Ocr],
    );

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(3, 4, 30, 9);
    feed(&mut instance, fake.select_region(region));

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(region));
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region)
        }]
    );
}

#[test]
fn result_panel_that_cannot_be_opened_is_reported() {
    let fake = single_display().with_window_launch_failing();
    let mut instance = instance_with_region_binding(
        &fake,
        vec![PostCaptureAction::CopyToClipboard, PostCaptureAction::Ocr],
    );

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(3, 4, 30, 9);
    let mut events = fake.select_region(region);
    let confirm = events.pop().unwrap();
    feed(&mut instance, events);
    let result = instance.handle(confirm);

    assert!(result.is_err());
    assert!(
        fake.clipboard().png_image().unwrap() == desktop_pattern(region),
        "打不开结果面板不影响复制"
    );
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "无法打开结果面板");
    assert!(
        notifications[0].body.contains("找不到程序文件"),
        "{notifications:?}"
    );
}

#[test]
fn translate_action_opens_one_result_panel_without_any_network_request() {
    let fake = single_display();
    let mut instance = instance_with_region_binding(
        &fake,
        vec![PostCaptureAction::Ocr, PostCaptureAction::Translate],
    );

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(3, 4, 30, 9);
    feed(&mut instance, fake.select_region(region));

    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region)
        }]
    );
    // 翻译只在用户在面板里点“翻译”时才联网。
    assert!(fake.http_requests().is_empty());
}
