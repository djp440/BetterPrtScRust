//! `bps` CLI：命令解析，以及把命令转交给常驻实例。
//!
//! CLI 一侧用 `FakeResidentChannel`，它把请求以平台事件的形式同步交给测试里的
//! `ResidentInstance`，所以每个测试都从 `bps` 的参数一路走到常驻实例的“命令”入口，
//! 再断言外部可见的结果：退出状态、输出，以及剪贴板等平台状态。

use std::cell::Cell;
use std::rc::Rc;

use bps_core::{
    CaptureMode, CaptureRequest, CliStatus, Command, HotkeyBinding, PostCaptureAction,
    ResidentInstance, default_bindings, run_cli,
};
use bps_platform::{Hotkey, Key, Modifiers, OverlayKey, PlatformEvent, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeResidentChannel};

fn primary_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary()
}

fn second_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5)
}

/// 两块显示器，鼠标在第二块上。
fn two_displays_cursor_on_second() -> FakePlatform {
    FakePlatform::new()
        .with_display(primary_display())
        .with_display(second_display())
        .with_cursor_at(Point::new(100, 20))
}

/// 用默认热键绑定启动的常驻实例；返回的闭包扮演它的平台事件循环。
fn resident_instance(fake: &FakePlatform) -> impl FnMut(bps_platform::PlatformEvent) + 'static {
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    move |event| {
        let _ = instance.handle(event);
    }
}

#[test]
fn bps_capture_display_copies_display_under_cursor_like_the_hotkey() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::running(resident_instance(&fake));

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert_eq!(channel.launches(), 0, "常驻实例已在运行时不应再拉起");
    // 与 Shift+PrtSc 的默认绑定一致：复制 + 自动存储。
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB", "Files"]);
    assert_eq!(clipboard.png_image().unwrap(), *second_display().image());
    assert_eq!(clipboard.files().unwrap(), fake.files());
}

#[test]
fn bps_capture_virtual_desktop_copies_all_displays_stitched_together() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::running(resident_instance(&fake));

    let outcome = run_cli(["capture", "virtual-desktop"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    let png = fake.clipboard().png_image().unwrap();
    // 主显示器 (0,0) 64x48 与副显示器 (64,-10) 96x54 的外接矩形是 (0,-10) 160x58。
    assert_eq!((png.width(), png.height()), (160, 58));
    assert_eq!(png.pixel(0, 10), primary_display().image().pixel(0, 0));
    assert_eq!(png.pixel(64, 0), second_display().image().pixel(0, 0));
    assert_eq!(png.pixel(0, 0), [0, 0, 0, 0], "主显示器上方的空隙应透明");
}

#[test]
fn bps_launches_resident_instance_when_not_running_then_completes_command() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::not_running(resident_instance(&fake));

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert_eq!(channel.launches(), 1);
    assert_eq!(
        fake.clipboard().png_image().unwrap(),
        *second_display().image()
    );
}

/// 默认绑定前面加一条“区域 + 复制”，`bps capture region` 因此只框选、复制，不进入标注。
fn bindings_with_region_copy() -> Vec<HotkeyBinding> {
    let region_copy = HotkeyBinding {
        hotkey: Hotkey::new(Modifiers::CTRL, Key::F(8)),
        capture: CaptureRequest::new(
            CaptureMode::Region,
            vec![PostCaptureAction::CopyToClipboard],
        ),
    };
    [vec![region_copy], default_bindings()].concat()
}

/// 带一条“区域 + 复制”绑定的常驻实例，外加坐在屏幕前的用户：每处理完一个事件，
/// 只要选区遮罩显示着，用户就在上面做 `gesture`（遮罩还开着就接着做下一步）。
fn resident_instance_with_user(
    fake: &FakePlatform,
    gesture: impl FnMut(&FakePlatform) -> Vec<PlatformEvent> + 'static,
) -> impl FnMut(PlatformEvent) + 'static {
    resident_instance_with_bindings_and_user(fake, bindings_with_region_copy(), gesture)
}

/// 用 `bindings` 启动的常驻实例，外加在遮罩上做 `gesture` 的用户。
fn resident_instance_with_bindings_and_user(
    fake: &FakePlatform,
    bindings: Vec<HotkeyBinding>,
    mut gesture: impl FnMut(&FakePlatform) -> Vec<PlatformEvent> + 'static,
) -> impl FnMut(PlatformEvent) + 'static {
    let mut instance = ResidentInstance::new(fake.platform(), bindings);
    assert!(instance.register_hotkeys().is_empty());
    let fake = fake.clone();
    move |event| {
        let _ = instance.handle(event);
        for _ in 0..10 {
            if !fake.overlay_visible() {
                break;
            }
            for event in gesture(&fake) {
                let _ = instance.handle(event);
            }
        }
    }
}

#[test]
fn bps_capture_region_replies_after_user_selects_region_on_overlay() {
    let fake = two_displays_cursor_on_second();
    let region = Rect::new(50, -5, 40, 30);
    let mut channel =
        FakeResidentChannel::running(resident_instance_with_user(&fake, move |fake| {
            fake.select_region(region)
        }));

    let outcome = run_cli(["capture", "region"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert!(!fake.overlay_visible());
    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (40, 30));
}

#[test]
fn bps_capture_region_reports_cancelled_when_user_presses_escape() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::running(resident_instance_with_user(&fake, |fake| {
        vec![fake.press_key(OverlayKey::Escape)]
    }));

    let outcome = run_cli(["capture", "region"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Cancelled, "{outcome:?}");
    assert_eq!(outcome.status.exit_code(), 5);
    assert!(outcome.stderr.contains("取消"), "{outcome:?}");
    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
    assert!(fake.files().is_empty());
}

#[test]
fn bps_capture_region_with_default_bindings_replies_after_user_finishes_annotating() {
    // 默认的 PrtSc 是“区域 + 标注”：框选后进入标注，用户点“复制”才结束。
    let fake = two_displays_cursor_on_second();
    let region = Rect::new(10, 5, 40, 30);
    let mut channel = FakeResidentChannel::running(resident_instance_with_bindings_and_user(
        &fake,
        default_bindings(),
        {
            let mut selected = false;
            move |fake| {
                if std::mem::replace(&mut selected, true) {
                    fake.click_text("复制")
                } else {
                    fake.drag(Point::new(10, 5), Point::new(49, 34))
                }
            }
        },
    ));

    let outcome = run_cli(["capture", "region"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert!(!fake.overlay_visible());
    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (region.width, region.height));
}

#[test]
fn bps_capture_fails_while_previous_capture_is_still_selecting() {
    let fake = two_displays_cursor_on_second();
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            vec![PostCaptureAction::CopyToClipboard],
        )))
        .unwrap();
    let shared = Rc::new(std::cell::RefCell::new(instance));
    let resident = shared.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = resident.borrow_mut().handle(event);
    });

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Failed, "{outcome:?}");
    assert!(
        outcome.stderr.contains("上一次捕获还没有结束"),
        "{outcome:?}"
    );
    assert!(fake.clipboard().is_empty());
    assert!(fake.overlay_visible(), "进行中的框选不受影响");
}

#[test]
fn bps_capture_window_is_accepted_but_reported_as_unsupported_until_implemented() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::running(resident_instance(&fake));

    let outcome = run_cli(["capture", "window"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Unsupported, "{outcome:?}");
    assert_eq!(outcome.status.exit_code(), 3);
    assert!(outcome.stderr.contains("暂不支持"), "{outcome:?}");
    assert!(fake.clipboard().is_empty());
}

#[test]
fn bps_capture_uses_post_capture_actions_of_first_hotkey_binding_with_that_mode() {
    let fake = two_displays_cursor_on_second();
    // 这条显示器绑定没有任何截后动作：按热键只截图、不复制，bps 也必须一样。
    let display_without_actions = HotkeyBinding {
        hotkey: Hotkey::new(Modifiers::CTRL, Key::F(9)),
        capture: CaptureRequest::new(CaptureMode::Display, Vec::new()),
    };
    let mut instance = ResidentInstance::new(
        fake.platform(),
        [vec![display_without_actions], default_bindings()].concat(),
    );
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert!(fake.clipboard().is_empty());
}

#[test]
fn bps_capture_copies_to_clipboard_when_no_hotkey_binding_uses_that_mode() {
    let fake = two_displays_cursor_on_second();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

#[test]
fn bps_capture_without_matching_binding_uses_global_output_settings() {
    let fake = two_displays_cursor_on_second().with_config_file("output_format = \"jpg\"\n");
    let mut instance = ResidentInstance::start(fake.platform());
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    assert_eq!(fake.clipboard().formats(), ["JPEG", "DIB"]);
}

#[test]
fn bps_reports_failure_when_resident_instance_cannot_complete_capture() {
    let no_displays = FakePlatform::new();
    let mut channel = FakeResidentChannel::running(resident_instance(&no_displays));

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Failed);
    assert_eq!(outcome.status.exit_code(), 1);
    assert!(outcome.stderr.contains("没有可用的显示器"), "{outcome:?}");
}

#[test]
fn bps_rejects_invalid_arguments_with_usage_without_contacting_resident_instance() {
    for args in [
        &[][..],
        &["capture"][..],
        &["capture", "fullscreen"][..],
        &["capture", "display", "extra"][..],
        &["record", "region"][..],
    ] {
        let requests = Rc::new(Cell::new(0));
        let counter = requests.clone();
        let mut channel = FakeResidentChannel::not_running(move |_| counter.set(counter.get() + 1));

        let outcome = run_cli(args, &mut channel);

        assert_eq!(outcome.status, CliStatus::Usage, "{args:?}: {outcome:?}");
        assert_eq!(outcome.status.exit_code(), 2);
        assert!(
            outcome.stderr.contains("bps capture <"),
            "{args:?}: {outcome:?}"
        );
        assert_eq!(channel.launches(), 0, "{args:?}");
        assert_eq!(requests.get(), 0, "{args:?}");
    }
}

#[test]
fn bps_help_lists_every_capture_mode() {
    for flag in ["--help", "-h", "help"] {
        let mut channel = FakeResidentChannel::not_running(|_| {});

        let outcome = run_cli([flag], &mut channel);

        assert_eq!(outcome.status, CliStatus::Success, "{flag}");
        for mode in ["display", "virtual-desktop", "region", "window"] {
            assert!(outcome.stdout.contains(mode), "{flag}: {}", outcome.stdout);
        }
        assert_eq!(channel.launches(), 0);
    }
}

#[test]
fn bps_reports_unreachable_when_resident_instance_cannot_be_launched() {
    let fake = two_displays_cursor_on_second();
    let mut channel = FakeResidentChannel::not_running(resident_instance(&fake)).launch_fails();

    let outcome = run_cli(["capture", "display"], &mut channel);

    assert_eq!(outcome.status, CliStatus::Unreachable);
    assert_eq!(outcome.status.exit_code(), 4);
    assert!(!outcome.stderr.is_empty());
    assert!(fake.clipboard().is_empty());
}
