//! 单实例：再次启动程序时，只把“激活”请求转交给已在运行的常驻实例，不产生第二个常驻实例。
//!
//! 谁是第二个进程由平台判定（Windows 上是命名管道已被占用）；这里测的是判定之后的转交。

use bps_core::{
    ForwardError, Reply, Request, ResidentInstance, WhenNotRunning, default_bindings, forward,
};
use bps_platform::{Hotkey, Key, Modifiers, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeResidentChannel};

#[test]
fn second_launch_activates_running_resident_instance_without_starting_another() {
    let fake = FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 8, 8)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });

    let reply = forward(&Request::Activate, &mut channel, WhenNotRunning::GiveUp);

    assert_eq!(reply, Ok(Reply::Done));
    assert_eq!(channel.launches(), 0);
    // 常驻实例照常运行：热键仍然注册着，也没有因为激活而捕获。
    assert_eq!(
        fake.registered_hotkeys(),
        [
            Hotkey::new(Modifiers::SHIFT, Key::PrintScreen),
            Hotkey::new(Modifiers::NONE, Key::PrintScreen),
        ]
    );
    assert!(fake.clipboard().is_empty());
}

#[test]
fn second_launch_does_not_start_a_resident_instance_when_the_running_one_is_gone() {
    let mut channel = FakeResidentChannel::not_running(|_| {});

    let reply = forward(&Request::Activate, &mut channel, WhenNotRunning::GiveUp);

    assert_eq!(reply, Err(ForwardError::NotRunning));
    assert_eq!(channel.launches(), 0);
}
