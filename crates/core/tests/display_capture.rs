//! 显示器捕获的端到端测试：从“命令”入口驱动，只断言外部可见的结果。
//!
//! 写法范例：用 `FakeDisplay` / `FakePlatform` 描述桌面，`fake.platform()` 交给
//! `ResidentInstance`，执行命令（或把假热键产生的事件交给常驻实例），
//! 最后通过 `fake` 检查剪贴板等外部可见状态。期望图像来自假显示器的画面，
//! 不由被测代码推导。

use std::path::Path;

use bps_core::{
    CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance, default_bindings,
};
use bps_platform::{Hotkey, Key, LocalDateTime, Modifiers, Point, Rect};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, decode};

/// 左边是 100% 缩放的主显示器；右边是 150% 缩放、向上错位 10 像素的副显示器。
fn primary_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary()
}

fn scaled_display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5)
}

fn copy_display_capture() -> Command {
    Command::Capture(CaptureRequest::new(
        CaptureMode::Display,
        vec![PostCaptureAction::CopyToClipboard],
    ))
}

#[test]
fn display_capture_copies_display_under_cursor_to_clipboard_as_png_and_dib() {
    let expected = scaled_display().image().clone();
    let fake = FakePlatform::new()
        .with_display(primary_display())
        .with_display(scaled_display())
        .with_cursor_at(Point::new(100, 20));
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance.execute(copy_display_capture()).unwrap();

    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    let png = clipboard.png_image().unwrap();
    // 按物理像素输出：150% 缩放的 96x54 显示器截出来仍是 96x54。
    assert_eq!((png.width(), png.height()), (96, 54));
    assert_eq!(png, expected);
    assert_eq!(clipboard.dib_image().unwrap(), expected);
}

#[test]
fn shift_print_screen_copies_and_auto_saves_display_under_cursor_with_default_bindings() {
    let expected = primary_display().image().clone();
    let fake = FakePlatform::new()
        .with_display(primary_display())
        .with_display(scaled_display())
        .with_cursor_at(Point::new(10, 10))
        .with_time(LocalDateTime::new(2026, 10, 24, 21, 30, 0));
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());

    let event = fake
        .press(Hotkey::new(Modifiers::SHIFT, Key::PrintScreen))
        .expect("默认绑定应注册 Shift+PrtSc");
    instance.handle(event).unwrap();

    let saved = Path::new(DEFAULT_PICTURES_DIR)
        .join("BetterPrtSc")
        .join("2026-10-24 21-30-00.png");
    assert_eq!(fake.files(), std::slice::from_ref(&saved));
    assert_eq!(decode::png(&fake.file(&saved).unwrap()), expected);
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB", "Files"]);
    assert_eq!(clipboard.png_image().unwrap(), expected);
    assert_eq!(clipboard.files().unwrap(), [saved]);
}
