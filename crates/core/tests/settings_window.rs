//! 打开设置界面：托盘菜单“设置”与再次启动程序（激活）都打开它；托盘菜单“退出”结束常驻实例。

use bps_core::{Flow, Reply, Request, ResidentInstance, WhenNotRunning, forward};
use bps_platform::StandaloneWindow;
use bps_platform_fake::{FakePlatform, FakeResidentChannel};

#[test]
fn tray_menu_settings_opens_the_settings_window() {
    let fake = FakePlatform::new();
    let mut instance = ResidentInstance::start(fake.platform());
    let registered = fake.registered_hotkeys();

    let flow = instance
        .handle(fake.choose_tray_menu_item("设置").unwrap())
        .unwrap();

    assert_eq!(flow, Flow::Continue);
    assert_eq!(fake.opened_windows(), [StandaloneWindow::Settings]);
    assert_eq!(fake.registered_hotkeys(), registered);
    assert!(fake.clipboard().is_empty());
}

#[test]
fn second_launch_opens_the_settings_window_of_the_running_instance() {
    let fake = FakePlatform::new();
    let mut instance = ResidentInstance::start(fake.platform());
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });

    let reply = forward(&Request::Activate, &mut channel, WhenNotRunning::GiveUp);

    assert_eq!(reply, Ok(Reply::Done));
    assert_eq!(fake.opened_windows(), [StandaloneWindow::Settings]);
}

#[test]
fn settings_window_that_cannot_be_opened_is_reported() {
    let fake = FakePlatform::new()
        .with_system_languages(&["en-US"])
        .with_window_launch_failing();
    let mut instance = ResidentInstance::start(fake.platform());

    let result = instance.handle(fake.choose_tray_menu_item("Settings").unwrap());

    assert!(result.is_err());
    let last = fake.notifications().pop().unwrap();
    assert_eq!(last.title, "Cannot open Settings");
    assert!(last.body.contains("找不到程序文件"), "{last:?}");
}

#[test]
fn tray_menu_exit_unregisters_hotkeys_and_exits() {
    let fake = FakePlatform::new();
    let mut instance = ResidentInstance::start(fake.platform());
    assert!(!fake.registered_hotkeys().is_empty());

    let flow = instance
        .handle(fake.choose_tray_menu_item("退出").unwrap())
        .unwrap();

    assert_eq!(flow, Flow::Exit);
    assert!(fake.registered_hotkeys().is_empty());
    assert!(fake.opened_windows().is_empty());
}
