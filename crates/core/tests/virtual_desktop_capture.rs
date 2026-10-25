//! 虚拟桌面捕获：所有显示器按各自的物理像素截取，再按它们在虚拟桌面里的位置拼接。
//!
//! 期望图像来自假平台的 `desktop_pattern`（每个虚拟桌面像素的颜色由坐标决定），
//! 没有任何显示器覆盖的空隙是透明像素；期望的尺寸是按布局手算的字面量。

use bps_core::{
    CaptureMode, CaptureRequest, Command, HotkeyBinding, PostCaptureAction, ResidentInstance,
    default_bindings,
};
use bps_platform::{Hotkey, Image, Key, Modifiers, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

/// 透明像素：没有任何显示器覆盖的空隙。
const GAP: [u8; 4] = [0, 0, 0, 0];

fn copy_virtual_desktop_capture() -> Command {
    Command::Capture(CaptureRequest::new(
        CaptureMode::VirtualDesktop,
        PostCaptureAction::CopyToClipboard,
    ))
}

fn platform_with(displays: &[FakeDisplay]) -> FakePlatform {
    displays
        .iter()
        .cloned()
        .fold(FakePlatform::new(), FakePlatform::with_display)
        .with_cursor_at(Point::new(1, 1))
}

/// `bounds` 范围内的期望截图：落在某块显示器内的像素取 `desktop_pattern`，其余透明。
fn expected_capture(bounds: Rect, displays: &[FakeDisplay]) -> Image {
    let pattern = desktop_pattern(bounds);
    let mut pixels = Vec::with_capacity(pattern.rgba().len());
    for row in 0..bounds.height {
        for col in 0..bounds.width {
            let point = Point::new(bounds.x + col as i32, bounds.y + row as i32);
            if displays.iter().any(|d| d.bounds().contains(point)) {
                pixels.extend_from_slice(&pattern.pixel(col, row));
            } else {
                pixels.extend_from_slice(&GAP);
            }
        }
    }
    Image::from_rgba(bounds.width, bounds.height, pixels)
}

/// 几种不规则布局，以及按布局手算的虚拟桌面范围。
fn layouts() -> Vec<(&'static str, Vec<FakeDisplay>, Rect)> {
    vec![
        (
            "右侧 150% 副显示器向上错位 10 像素",
            vec![
                FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary(),
                FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5),
            ],
            Rect::new(0, -10, 160, 58),
        ),
        (
            "左侧 125% 副显示器在负坐标且向下错位",
            vec![
                FakeDisplay::new(Rect::new(-80, 20, 80, 45)).scale_factor(1.25),
                FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary(),
            ],
            Rect::new(-80, 0, 144, 65),
        ),
        (
            "上方 200% 窄显示器居中、右侧显示器底对齐",
            vec![
                FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary(),
                FakeDisplay::new(Rect::new(8, -36, 48, 36)).scale_factor(2.0),
                FakeDisplay::new(Rect::new(64, 16, 32, 32)),
            ],
            Rect::new(0, -36, 96, 84),
        ),
    ]
}

#[test]
fn virtual_desktop_capture_stitches_displays_with_mixed_scale_factors_at_their_physical_positions()
{
    for (layout, displays, bounds) in layouts() {
        let fake = platform_with(&displays);
        let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

        instance.execute(copy_virtual_desktop_capture()).unwrap();

        let clipboard = fake.clipboard();
        assert_eq!(clipboard.formats(), ["PNG", "DIB"], "{layout}");
        let png = clipboard.png_image().unwrap();
        assert_eq!(
            (png.width(), png.height()),
            (bounds.width, bounds.height),
            "{layout}"
        );
        let expected = expected_capture(bounds, &displays);
        assert!(png == expected, "{layout}：拼接结果与期望画面不符");
        assert!(
            clipboard.dib_image().unwrap() == expected,
            "{layout}：DIB 与期望画面不符"
        );
    }
}

#[test]
fn hotkey_binding_with_virtual_desktop_mode_copies_virtual_desktop() {
    let (_, displays, bounds) = layouts().remove(0);
    let fake = platform_with(&displays);
    let hotkey = Hotkey::new(Modifiers::CTRL.with(Modifiers::SHIFT), Key::F(12));
    let binding = HotkeyBinding {
        hotkey,
        capture: CaptureRequest::new(
            CaptureMode::VirtualDesktop,
            PostCaptureAction::CopyToClipboard,
        ),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    assert!(instance.register_hotkeys().is_empty());

    instance.handle(fake.press(hotkey).unwrap()).unwrap();

    let png = fake.clipboard().png_image().unwrap();
    assert!(png == expected_capture(bounds, &displays));
}

#[test]
fn default_bindings_do_not_bind_virtual_desktop() {
    assert!(
        default_bindings()
            .iter()
            .all(|binding| binding.capture.mode != CaptureMode::VirtualDesktop)
    );
}
