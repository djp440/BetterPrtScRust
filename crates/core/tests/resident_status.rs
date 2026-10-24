//! 设置界面经命名管道向常驻实例查询状态：哪些热键绑定注册失败及原因、检查更新的结果。

use bps_core::{
    BindingFailure, Locale, RegistrationError, Reply, Request, ResidentInstance, ResidentStatus,
    Texts, UpdateStatus, WhenNotRunning, forward,
};
use bps_platform::{Hotkey, Key, Modifiers};
use bps_platform_fake::{FakePlatform, FakeResidentChannel};

const THREE_BINDINGS: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
actions = ["copy_to_clipboard"]

[[bindings]]
hotkey = "Ctrl+F10"
mode = "display"
actions = ["copy_to_clipboard"]

[[bindings]]
hotkey = "Ctrl+F10"
mode = "virtual_desktop"
actions = ["copy_to_clipboard"]
"#;

fn ctrl(key: Key) -> Hotkey {
    Hotkey::new(Modifiers::CTRL, key)
}

/// 像设置界面那样，经通道向常驻实例查询状态。
fn query_status(instance: ResidentInstance) -> (ResidentStatus, ResidentInstance) {
    let shared = std::rc::Rc::new(std::cell::RefCell::new(Some(instance)));
    let inside = shared.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = inside.borrow_mut().as_mut().unwrap().handle(event);
    });
    let reply = forward(&Request::Status, &mut channel, WhenNotRunning::GiveUp).unwrap();
    let Reply::Status(status) = reply else {
        panic!("应回复状态：{reply:?}");
    };
    drop(channel);
    let instance = shared.borrow_mut().take().unwrap();
    (status, instance)
}

#[test]
fn status_lists_failed_bindings_with_reasons() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(ctrl(Key::F(9)))
        .with_config_file(THREE_BINDINGS);
    let instance = ResidentInstance::start(fake.platform());
    let notifications_before = fake.notifications().len();

    let (status, _) = query_status(instance);

    assert_eq!(
        status.registration_failures,
        [
            BindingFailure {
                index: 0,
                hotkey: ctrl(Key::F(9)),
                error: RegistrationError::TakenByOtherProgram,
            },
            BindingFailure {
                index: 2,
                hotkey: ctrl(Key::F(10)),
                error: RegistrationError::DuplicateOf(1),
            },
        ]
    );
    // 查询状态只是读取：不打开窗口、不发通知。
    assert!(fake.opened_windows().is_empty());
    assert_eq!(fake.notifications().len(), notifications_before);
}

#[test]
fn failure_reasons_follow_the_interface_language() {
    let en = Texts::of(Locale::En);
    let zh = Texts::of(Locale::ZhCn);

    assert_eq!(
        RegistrationError::TakenByOtherProgram.localized(en),
        "Already in use by another program"
    );
    assert_eq!(
        RegistrationError::DuplicateOf(1).localized(en),
        "Same hotkey as binding 2"
    );
    assert_eq!(
        RegistrationError::DuplicateOf(1).localized(zh),
        "与第 2 条热键绑定的热键重复"
    );
}

#[test]
fn status_reflects_the_reloaded_configuration() {
    let fake = FakePlatform::new()
        .with_hotkey_taken_by_other_program(ctrl(Key::F(9)))
        .with_config_file(THREE_BINDINGS);
    let mut instance = ResidentInstance::start(fake.platform());
    let fixed = THREE_BINDINGS
        .replacen("Ctrl+F9", "Ctrl+F8", 1)
        .replacen("virtual_desktop", "display", 1)
        .replacen(
            "Ctrl+F10\"\nmode = \"display",
            "Ctrl+F11\"\nmode = \"display",
            1,
        );
    instance
        .handle(fake.edit_config_file(&fixed).unwrap())
        .unwrap();

    let (status, _) = query_status(instance);

    assert_eq!(status.registration_failures, []);
}

#[test]
fn update_check_is_reported_as_disabled_by_default() {
    let fake = FakePlatform::new();
    let instance = ResidentInstance::start(fake.platform());

    let (status, _) = query_status(instance);

    assert_eq!(status.update, UpdateStatus::Disabled);
}

#[test]
fn system_error_text_with_tabs_and_line_breaks_survives_the_channel() {
    let status = ResidentStatus {
        registration_failures: vec![BindingFailure {
            index: 3,
            hotkey: ctrl(Key::F(12)),
            error: RegistrationError::Platform(bps_platform::PlatformError::new(
                "注册失败：\t参数错误\n(0x80070057)",
            )),
        }],
        update: UpdateStatus::Failed("无法连接\tGitHub".to_owned()),
    };
    let fake = FakePlatform::new();
    let mut channel = FakeResidentChannel::running(move |event| {
        if let bps_platform::PlatformEvent::RequestReceived(request) = event {
            request.reply(Reply::Status(status.clone()).encode());
        }
    });
    drop(fake);

    let reply = forward(&Request::Status, &mut channel, WhenNotRunning::GiveUp).unwrap();

    let Reply::Status(received) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(received.registration_failures[0].index, 3);
    assert_eq!(
        received.registration_failures[0].error,
        RegistrationError::Platform(bps_platform::PlatformError::new(
            "注册失败： 参数错误 (0x80070057)"
        ))
    );
    assert_eq!(
        received.update,
        UpdateStatus::Failed("无法连接 GitHub".to_owned())
    );
}
