//! 所有截后动作都已实现后，只剩“默认绑定要么能跑、要么明确报告暂不支持”这一条兜底。

use bps_core::{CoreError, Flow, ResidentInstance};
use bps_platform::{Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_cursor_at(Point::new(5, 5))
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
