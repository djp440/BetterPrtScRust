//! 常驻实例生命周期：热键注册与退出。

use bps_core::{Flow, ResidentInstance, default_bindings};
use bps_platform::{Hotkey, Key, Modifiers, PlatformEvent, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

#[test]
fn exit_request_releases_hotkeys_and_stops_resident_instance() {
    let fake = FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 8, 8)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    assert_eq!(
        fake.registered_hotkeys(),
        [Hotkey::new(Modifiers::SHIFT, Key::PrintScreen)]
    );

    assert_eq!(
        instance.handle(PlatformEvent::ExitRequested),
        Ok(Flow::Exit)
    );
    assert!(fake.registered_hotkeys().is_empty());
}
