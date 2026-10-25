//! 设置界面录制热键期间，经命名管道暂停常驻实例的全局热键；恢复，或租约到期后自动恢复。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use bps_core::{
    CaptureMode, CaptureRequest, HotkeyBinding, PostCaptureAction, Reply, Request,
    ResidentInstance, WhenNotRunning, forward,
};
use bps_platform::{Hotkey, Key, Modifiers, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeResidentChannel};

fn hotkey() -> Hotkey {
    Hotkey::new(Modifiers::CTRL, Key::F(9))
}

fn setup() -> (FakePlatform, Rc<RefCell<ResidentInstance>>) {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 32, 24)).primary())
        .with_cursor_at(Point::new(1, 1));
    let mut instance = ResidentInstance::new(
        fake.platform(),
        vec![HotkeyBinding {
            hotkey: hotkey(),
            capture: CaptureRequest::new(CaptureMode::Display, PostCaptureAction::CopyToClipboard),
        }],
    );
    assert!(instance.register_hotkeys().is_empty());
    (fake, Rc::new(RefCell::new(instance)))
}

fn send(instance: &Rc<RefCell<ResidentInstance>>, request: &Request) -> Reply {
    let inside = instance.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = inside.borrow_mut().handle(event);
    });
    forward(request, &mut channel, WhenNotRunning::GiveUp).unwrap()
}

fn press(fake: &FakePlatform, instance: &Rc<RefCell<ResidentInstance>>) {
    let event = fake.press(hotkey()).expect("热键应保持注册");
    let _ = instance.borrow_mut().handle(event);
}

#[test]
fn pause_and_resume_requests_round_trip_on_the_channel() {
    for request in [Request::PauseHotkeys, Request::ResumeHotkeys] {
        assert_eq!(Request::decode(&request.encode()), Some(request));
    }
}

#[test]
fn hotkeys_do_nothing_while_paused_and_work_again_after_resume() {
    let (fake, instance) = setup();

    assert_eq!(send(&instance, &Request::PauseHotkeys), Reply::Done);
    press(&fake, &instance);
    assert!(fake.clipboard().formats().is_empty(), "暂停期间不应截图");

    assert_eq!(send(&instance, &Request::ResumeHotkeys), Reply::Done);
    press(&fake, &instance);
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

#[test]
fn pause_expires_by_itself_when_the_settings_process_never_resumes() {
    let (fake, instance) = setup();
    send(&instance, &Request::PauseHotkeys);

    // 设置进程崩溃：既不续租也不恢复。
    fake.advance_clock(Duration::from_secs(60));
    press(&fake, &instance);

    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

#[test]
fn renewing_the_pause_keeps_hotkeys_paused() {
    let (fake, instance) = setup();
    send(&instance, &Request::PauseHotkeys);
    for _ in 0..5 {
        fake.advance_clock(Duration::from_secs(1));
        send(&instance, &Request::PauseHotkeys);
    }
    press(&fake, &instance);
    assert!(fake.clipboard().formats().is_empty());
}
