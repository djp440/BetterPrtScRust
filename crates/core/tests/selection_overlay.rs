//! 选区遮罩上画了什么：选区外变暗、选区边框、物理像素尺寸标签、操作提示，
//! 以及它们按所在显示器的缩放比例放大。
//!
//! 断言的是假遮罩上最近一次画出的场景（`FakePlatform::overlay_scene`），即用户看到的内容。

use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
use bps_platform::{
    Corner, Modifiers, OverlayCursor, OverlayInput, OverlayItem, OverlayScene, PlatformEvent,
    Point, PointerButton, Rect,
};
use bps_platform_fake::{FakeDisplay, FakePlatform};

/// 左边 100% 的主显示器，右边 200% 的副显示器。
fn mixed_scale_desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 200, 150)).primary())
        .with_display(FakeDisplay::new(Rect::new(200, 0, 400, 300)).scale_factor(2.0))
        .with_cursor_at(Point::new(10, 10))
}

const VIRTUAL_DESKTOP: Rect = Rect::new(0, 0, 600, 300);

fn selecting(fake: &FakePlatform) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            vec![PostCaptureAction::CopyToClipboard],
        )))
        .unwrap();
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 按下左键从 `from` 拖到 `to`，但还没有松开。
fn drag_without_release(fake: &FakePlatform, from: Point, to: Point) -> Vec<PlatformEvent> {
    vec![
        fake.overlay_input(OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: from,
            modifiers: Modifiers::NONE,
            double_click: false,
        }),
        fake.overlay_input(OverlayInput::PointerMoved {
            position: to,
            modifiers: Modifiers::NONE,
        }),
    ]
}

fn fills(scene: &OverlayScene) -> Vec<Rect> {
    scene
        .items
        .iter()
        .filter_map(|item| match item {
            OverlayItem::Fill { rect, color } if color.a > 0 => Some(*rect),
            _ => None,
        })
        .collect()
}

fn frames(scene: &OverlayScene) -> Vec<(Rect, u32)> {
    scene
        .items
        .iter()
        .filter_map(|item| match item {
            OverlayItem::Frame { rect, width, .. } => Some((*rect, *width)),
            _ => None,
        })
        .collect()
}

fn label_texts(scene: &OverlayScene) -> Vec<String> {
    scene.labels().map(|label| label.text.clone()).collect()
}

fn area(rect: Rect) -> u64 {
    u64::from(rect.width) * u64::from(rect.height)
}

#[test]
fn whole_virtual_desktop_is_dimmed_before_anything_is_selected() {
    let fake = mixed_scale_desktop();
    let _instance = selecting(&fake);

    let scene = fake.overlay_scene().unwrap();
    assert_eq!(fills(&scene), [VIRTUAL_DESKTOP]);
    assert!(frames(&scene).is_empty());
    assert!(label_texts(&scene).is_empty());
    assert_eq!(scene.cursor, OverlayCursor::Crosshair);
}

#[test]
fn selection_is_left_undimmed_and_framed_from_outside() {
    let fake = mixed_scale_desktop();
    let mut instance = selecting(&fake);
    let region = Rect::new(20, 40, 100, 50);

    feed(
        &mut instance,
        fake.drag(Point::new(20, 40), Point::new(119, 89)),
    );

    let scene = fake.overlay_scene().unwrap();
    let dimmed = fills(&scene);
    assert!(
        dimmed
            .iter()
            .all(|rect| rect.intersection(region).is_none()),
        "选区内不应变暗：{dimmed:?}"
    );
    assert_eq!(
        dimmed.iter().map(|rect| area(*rect)).sum::<u64>(),
        area(VIRTUAL_DESKTOP) - area(region),
        "选区外应全部变暗且不重叠：{dimmed:?}"
    );
    assert_eq!(frames(&scene), [(region, 1)]);
}

#[test]
fn size_label_shows_physical_pixels_while_dragging() {
    let fake = mixed_scale_desktop();
    let mut instance = selecting(&fake);

    // 在 200% 显示器上拖出 30 × 20 物理像素（15 × 10 逻辑像素）。
    feed(
        &mut instance,
        drag_without_release(&fake, Point::new(300, 100), Point::new(329, 119)),
    );

    let scene = fake.overlay_scene().unwrap();
    assert_eq!(label_texts(&scene), ["30 × 20"], "拖拽中只显示尺寸");
}

#[test]
fn labels_and_frame_scale_with_display_under_selection() {
    let fake = mixed_scale_desktop();
    let mut instance = selecting(&fake);
    feed(
        &mut instance,
        drag_without_release(&fake, Point::new(20, 100), Point::new(60, 120)),
    );
    let on_primary = fake.overlay_scene().unwrap();
    feed(
        &mut instance,
        vec![fake.overlay_input(OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: Point::new(60, 120),
            modifiers: Modifiers::NONE,
        })],
    );
    feed(
        &mut instance,
        drag_without_release(&fake, Point::new(300, 100), Point::new(340, 120)),
    );
    let on_secondary = fake.overlay_scene().unwrap();

    let font = |scene: &OverlayScene| scene.labels().next().unwrap().font_size;
    assert_eq!(font(&on_primary), 13.0);
    assert_eq!(font(&on_secondary), 26.0);
    assert_eq!(frames(&on_primary)[0].1, 1);
    assert_eq!(frames(&on_secondary)[0].1, 2);
}

#[test]
fn hint_tells_how_to_finish_once_region_is_selected() {
    let fake = mixed_scale_desktop();
    let mut instance = selecting(&fake);

    feed(
        &mut instance,
        fake.drag(Point::new(20, 40), Point::new(119, 89)),
    );

    let texts = label_texts(&fake.overlay_scene().unwrap());
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert_eq!(texts[0], "100 × 50");
    assert!(
        texts[1].contains("Enter") && texts[1].contains("Esc"),
        "{texts:?}"
    );
}

#[test]
fn size_label_sits_above_selection_and_moves_inside_at_top_edge_of_display() {
    let fake = mixed_scale_desktop();
    let mut instance = selecting(&fake);

    feed(
        &mut instance,
        drag_without_release(&fake, Point::new(20, 100), Point::new(60, 120)),
    );
    let label = fake
        .overlay_scene()
        .unwrap()
        .labels()
        .next()
        .unwrap()
        .clone();
    assert_eq!(label.corner, Corner::BottomLeft);
    assert!(label.anchor.y < 100 && label.anchor.x == 20, "{label:?}");

    feed(
        &mut instance,
        vec![fake.overlay_input(OverlayInput::PointerMoved {
            position: Point::new(60, 0),
            modifiers: Modifiers::NONE,
        })],
    );
    // 选区现在是 (20, 0)–(60, 100)，上方没有空间。
    let label = fake
        .overlay_scene()
        .unwrap()
        .labels()
        .next()
        .unwrap()
        .clone();
    assert_eq!(label.corner, Corner::TopLeft);
    assert!(
        label.anchor.y > 0 && label.anchor.x > 20,
        "标签应在选区内：{label:?}"
    );
}
