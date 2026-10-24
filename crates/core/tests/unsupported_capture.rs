//! 配置能表达后续 ticket 才实现的截后动作；执行到它们时明确报告“暂不支持”，
//! 不崩溃、不产生任何产物。

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, Flow, PostCaptureAction, ResidentInstance,
};
use bps_platform::{Hotkey, Key, Modifiers, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_cursor_at(Point::new(5, 5))
}

#[test]
fn unimplemented_action_rejects_whole_capture_before_earlier_actions_run() {
    let fake = desktop();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    let result = instance.execute(Command::Capture(CaptureRequest::new(
        CaptureMode::Display,
        vec![
            PostCaptureAction::CopyToClipboard,
            PostCaptureAction::LongScreenshot,
        ],
    )));

    assert_eq!(
        result,
        Err(CoreError::UnsupportedAction(
            PostCaptureAction::LongScreenshot
        ))
    );
    assert!(fake.clipboard().is_empty(), "不应只执行了一半的截后动作");
}

#[test]
fn pressing_unimplemented_default_binding_notifies_unsupported_and_keeps_running() {
    let fake = desktop();
    let mut instance = ResidentInstance::start(fake.platform());

    let long_screenshot = fake
        .press(Hotkey::new(
            Modifiers::CTRL.with(Modifiers::SHIFT),
            Key::PrintScreen,
        ))
        .expect("默认绑定应注册 Ctrl+Shift+PrtSc");
    let result = instance.handle(long_screenshot);

    // 默认的 Ctrl+Shift+PrtSc 是“区域 + 长截屏”：长截屏还没实现，在打开选区遮罩之前就整体拒绝。
    assert_eq!(
        result,
        Err(CoreError::UnsupportedAction(
            PostCaptureAction::LongScreenshot
        ))
    );
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "暂不支持");
    assert_eq!(notifications[0].body, "暂不支持“长截屏”截后动作");
    assert!(!fake.overlay_visible(), "不应打开选区遮罩");
    assert!(fake.clipboard().is_empty());
    assert_eq!(
        instance.handle(bps_platform::PlatformEvent::ExitRequested),
        Ok(Flow::Exit)
    );
}

#[test]
fn every_default_binding_either_runs_or_reports_unsupported() {
    let fake = desktop();
    let mut instance = ResidentInstance::start(fake.platform());

    for hotkey in fake.registered_hotkeys() {
        let result = instance.handle(fake.press(hotkey).unwrap());
        // 打开了选区遮罩的（区域 + 标注）先取消，再试下一个。
        if fake.overlay_visible() {
            instance
                .handle(fake.press_key(bps_platform::OverlayKey::Escape))
                .unwrap();
        }
        assert!(
            matches!(
                result,
                Ok(Flow::Continue)
                    | Err(CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_))
            ),
            "{hotkey}: {result:?}"
        );
        // 需要在选区遮罩上操作的捕获（如 Ctrl+PrtSc 的窗口）已经开始：取消它，再试下一个热键。
        if fake.overlay_visible() {
            let escape = fake.press_key(bps_platform::OverlayKey::Escape);
            assert_eq!(instance.handle(escape), Ok(Flow::Continue), "{hotkey}");
        }
    }
}
