//! 跨显示器拼接的公共 API：按虚拟桌面坐标矩形截图（`capture_rect`），
//! 以及把已截下的各显示器画面按位置拼接、裁剪（`stitch`）。区域截图会复用它们。
//!
//! 期望图像来自假平台的 `desktop_pattern`，与显示器如何划分无关。

use bps_core::{DesktopImage, capture_rect, stitch, virtual_desktop_bounds};
use bps_platform::{FrameSource, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

/// 左边 100% 主显示器；右边 150% 副显示器，向上错位 10 像素。
fn displays() -> [FakeDisplay; 2] {
    [
        FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary(),
        FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5),
    ]
}

fn frame_source() -> Box<dyn FrameSource> {
    let [primary, scaled] = displays();
    FakePlatform::new()
        .with_display(primary)
        .with_display(scaled)
        .platform()
        .frame_source
}

/// 虚拟桌面上 `(x, y)` 处应有的像素。
fn desktop_pixel(x: i32, y: i32) -> [u8; 4] {
    desktop_pattern(Rect::new(x, y, 1, 1)).pixel(0, 0)
}

#[test]
fn capture_rect_stitches_rect_spanning_displays_with_different_scale_factors() {
    let rect = Rect::new(40, 5, 60, 30);

    let image = capture_rect(frame_source().as_ref(), rect).unwrap();

    assert_eq!((image.width(), image.height()), (60, 30));
    assert!(
        image == desktop_pattern(rect),
        "跨显示器拼接结果与期望画面不符"
    );
}

#[test]
fn capture_rect_leaves_parts_outside_every_display_transparent() {
    let rect = Rect::new(-8, -12, 80, 20);

    let image = capture_rect(frame_source().as_ref(), rect).unwrap();

    assert_eq!((image.width(), image.height()), (80, 20));
    assert_eq!(image.pixel(0, 0), [0, 0, 0, 0], "虚拟桌面左上方之外");
    assert_eq!(image.pixel(30, 5), [0, 0, 0, 0], "主显示器上方的空隙");
    assert_eq!(image.pixel(8, 12), desktop_pixel(0, 0), "主显示器左上角");
    assert_eq!(image.pixel(72, 2), desktop_pixel(64, -10), "副显示器左上角");
    assert_eq!(image.pixel(79, 19), desktop_pixel(71, 7));
}

#[test]
fn virtual_desktop_bounds_is_bounding_rect_of_all_displays() {
    let displays = frame_source().displays().unwrap();

    assert_eq!(
        virtual_desktop_bounds(&displays),
        Some(Rect::new(0, -10, 160, 58))
    );
    assert_eq!(virtual_desktop_bounds(&[]), None);
}

#[test]
fn stitch_crops_rect_from_display_images_already_captured() {
    // 例如冻结帧：先把每块显示器的画面截下来，用户框选后再从中拼出区域。
    let frozen: Vec<DesktopImage> = displays()
        .iter()
        .map(|display| DesktopImage {
            origin: Point::new(display.bounds().x, display.bounds().y),
            image: display.image().clone(),
        })
        .collect();
    let rect = Rect::new(60, -2, 10, 8);

    let image = stitch(rect, &frozen);

    assert_eq!(image.pixel(0, 0), [0, 0, 0, 0], "主显示器上方的空隙");
    assert_eq!(image.pixel(0, 2), desktop_pixel(60, 0));
    assert_eq!(image.pixel(4, 0), desktop_pixel(64, -2));
    assert_eq!(image.pixel(9, 7), desktop_pixel(69, 5));
}
