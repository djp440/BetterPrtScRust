//! 显示器捕获的端到端测试：从“命令”入口驱动，只断言外部可见的结果。
//!
//! 写法范例：用 `FakeDisplay` / `FakePlatform` 描述桌面，`fake.platform()` 交给
//! `ResidentInstance`，执行命令（或把假热键产生的事件交给常驻实例），
//! 最后通过 `fake` 检查剪贴板等外部可见状态。期望图像来自假显示器的画面，
//! 不由被测代码推导。

use bps_core::{
    CaptureMode, CaptureRequest, Command, HotkeyBinding, PostCaptureAction, ResidentInstance,
};
use bps_platform::{Hotkey, Key, Modifiers, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform};

/// 左边是 100% 缩放的主显示器；右边是 150% 缩放、向上错位 10 像素的副显示器。
fn primary_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary()
}

fn scaled_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5)
}

fn copy_display_capture() -> CaptureRequest {
    CaptureRequest::new(
        CaptureMode::Display,
        vec![PostCaptureAction::CopyToClipboard],
    )
}

#[test]
fn display_capture_copies_display_under_cursor_to_clipboard_as_png_and_dib() {
    let expected = scaled_display().image().clone();
    let fake = FakePlatform::new()
        .with_display(primary_display())
        .with_display(scaled_display())
        .with_cursor_at(Point::new(100, 20));
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(Command::Capture(copy_display_capture()))
        .unwrap();

    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    let png = clipboard.png_image().unwrap();
    // 按物理像素输出：150% 缩放的 96x54 显示器截出来仍是 96x54。
    assert_eq!((png.width(), png.height()), (96, 54));
    assert_eq!(png, expected);
    assert_eq!(clipboard.dib_image().unwrap(), expected);
}

#[test]
fn hotkey_bound_to_display_capture_copies_display_under_cursor() {
    let expected = primary_display().image().clone();
    let fake = FakePlatform::new()
        .with_display(primary_display())
        .with_display(scaled_display())
        .with_cursor_at(Point::new(10, 10));
    let shift_print_screen = Hotkey::new(Modifiers::SHIFT, Key::PrintScreen);
    let binding = HotkeyBinding {
        hotkey: shift_print_screen,
        capture: copy_display_capture(),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    assert!(instance.register_hotkeys().is_empty());

    let event = fake.press(shift_print_screen).expect("绑定的热键应已注册");
    instance.handle(event).unwrap();

    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    assert_eq!(clipboard.png_image().unwrap(), expected);
}
