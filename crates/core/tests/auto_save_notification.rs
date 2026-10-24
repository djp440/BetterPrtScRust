//! 自动存储完成（或失败）后发出的通知。

use std::path::{Path, PathBuf};

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, HotkeyBinding, PostCaptureAction,
    ResidentInstance, SaveDir,
};
use bps_platform::{Hotkey, Key, LocalDateTime, Modifiers, NotificationClick, Rect};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform};

fn fake() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7))
}

fn capture(actions: Vec<PostCaptureAction>) -> CaptureRequest {
    CaptureRequest::new(CaptureMode::Display, actions)
}

#[test]
fn auto_save_notifies_and_click_opens_saved_file() {
    let fake = fake();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(Command::Capture(capture(vec![PostCaptureAction::AutoSave])))
        .unwrap();

    let saved = Path::new(DEFAULT_PICTURES_DIR)
        .join("BetterPrtSc")
        .join("2026-10-24 09-05-07.png");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "截图已保存");
    assert!(
        notifications[0].body.contains("2026-10-24 09-05-07.png"),
        "通知正文应包含文件名：{}",
        notifications[0].body
    );
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenFile(saved))
    );
}

#[test]
fn copy_without_auto_save_sends_no_notification() {
    let fake = fake();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(Command::Capture(capture(vec![
            PostCaptureAction::CopyToClipboard,
        ])))
        .unwrap();

    assert!(fake.notifications().is_empty());
}

#[test]
fn failed_auto_save_notifies_failure_but_still_copies_image() {
    let read_only = PathBuf::from(r"D:\只读");
    let fake = fake().with_read_only_dir(&read_only);
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    let mut request = capture(vec![
        PostCaptureAction::CopyToClipboard,
        PostCaptureAction::AutoSave,
    ]);
    request.output.save_dir = SaveDir::Custom(read_only.join("截图"));

    let result = instance.execute(Command::Capture(request));

    assert!(
        matches!(result, Err(CoreError::Platform(_))),
        "存储失败应返回错误：{result:?}"
    );
    assert!(fake.files().is_empty());
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "自动存储失败");
    assert!(
        notifications[0].body.contains("只读"),
        "{}",
        notifications[0].body
    );
    assert_eq!(notifications[0].on_click, None);
}

#[test]
fn failed_auto_save_triggered_by_hotkey_is_notified_only_once() {
    let read_only = PathBuf::from(r"D:\只读");
    let fake = fake().with_read_only_dir(&read_only);
    let ctrl_f9 = Hotkey::new(Modifiers::CTRL, Key::F(9));
    let mut request = capture(vec![
        PostCaptureAction::CopyToClipboard,
        PostCaptureAction::AutoSave,
    ]);
    request.output.save_dir = SaveDir::Custom(read_only.join("截图"));
    let binding = HotkeyBinding {
        hotkey: ctrl_f9,
        capture: request,
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    assert!(instance.register_hotkeys().is_empty());

    let result = instance.handle(fake.press(ctrl_f9).unwrap());

    assert!(result.is_err(), "{result:?}");
    let titles: Vec<_> = fake.notifications().into_iter().map(|n| n.title).collect();
    assert_eq!(titles, ["自动存储失败"]);
}
