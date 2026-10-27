//! 默认绑定的冒烟：每个默认热键都能触发一次捕获而不出错。

use bps_core::{Flow, ResidentInstance};
use bps_platform::{Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_cursor_at(Point::new(5, 5))
}

#[test]
fn every_default_binding_starts_its_capture() {
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
        assert_eq!(result, Ok(Flow::Continue), "{hotkey}");
        // 需要在选区遮罩上操作的捕获（如 Ctrl+PrtSc 的区域）已经开始：取消它，再试下一个热键。
        if fake.overlay_visible() {
            let escape = fake.press_key(bps_platform::OverlayKey::Escape);
            assert_eq!(instance.handle(escape), Ok(Flow::Continue), "{hotkey}");
        }
    }
}
