//! 放大镜：框选时光标旁边显示光标处放大的像素网格和这个像素的颜色值；按 C 把颜色值以
//! `#RRGGBB` 复制到剪贴板。
//!
//! 断言的是假遮罩上最近一次画出的场景。放大的像素是颜色等于冻结帧像素的正方形填充，
//! 期望颜色来自假平台的 `desktop_pattern`（每个像素颜色唯一）。

use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
use bps_platform::{
    Color, Modifiers, OverlayInput, OverlayItem, OverlayKey, OverlayScene, PlatformEvent, Point,
    PointerButton, Rect,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_cursor_at(Point::new(10, 10))
}

fn selecting(fake: &FakePlatform) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::CopyToClipboard,
        )))
        .unwrap();
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn move_to(fake: &FakePlatform, instance: &mut ResidentInstance, position: Point) {
    feed(
        instance,
        vec![fake.overlay_input(OverlayInput::PointerMoved {
            position,
            modifiers: Modifiers::NONE,
        })],
    );
}

/// 冻结帧（`desktop_pattern`）里 `point` 处像素的颜色。
fn color_at(point: Point) -> Color {
    let [r, g, b, a] = desktop_pattern(Rect::new(point.x, point.y, 1, 1)).pixel(0, 0);
    Color::rgba(r, g, b, a)
}

fn hex(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

/// 放大镜里 `center` 这个像素放大后的格子：颜色等于该像素的正方形填充，并且它周围
/// `radius` 圈的像素也各有一个同样大小的格子、按像素的相对位置排成网格。没有时返回 `None`。
fn magnified_cell(scene: &OverlayScene, center: Point, radius: i32) -> Option<Rect> {
    let cells_of = |color: Color| {
        scene
            .items
            .iter()
            .filter_map(move |item| match item {
                OverlayItem::Fill { rect, color: c }
                    if *c == color && rect.width == rect.height =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    cells_of(color_at(center)).into_iter().find(|cell| {
        let size = cell.width as i32;
        (-radius..=radius).all(|dy| {
            (-radius..=radius).all(|dx| {
                let expected = Rect::new(
                    cell.x + dx * size,
                    cell.y + dy * size,
                    cell.width,
                    cell.height,
                );
                cells_of(color_at(Point::new(center.x + dx, center.y + dy))).contains(&expected)
            })
        })
    })
}

fn distance(rect: Rect, point: Point) -> i32 {
    let dx = (rect.x - point.x)
        .max(point.x - (rect.x + rect.width as i32 - 1))
        .max(0);
    let dy = (rect.y - point.y)
        .max(point.y - (rect.y + rect.height as i32 - 1))
        .max(0);
    dx.max(dy)
}

#[test]
fn magnifier_beside_pointer_shows_enlarged_pixels_and_color_value() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    let pointer = Point::new(300, 200);

    move_to(&fake, &mut instance, pointer);

    let scene = fake.overlay_scene().unwrap();
    let cell = magnified_cell(&scene, pointer, 3).expect("放大镜应显示光标周围放大的像素网格");
    assert!(cell.width >= 4, "像素应被放大：{cell:?}");
    assert!(
        !cell.contains(pointer),
        "放大镜不应挡住光标处的像素：{cell:?}"
    );
    assert!(
        distance(cell, pointer) <= 200,
        "放大镜应在光标旁边：{cell:?}"
    );
    assert!(
        scene.text_rect(&hex(color_at(pointer))).is_some(),
        "应显示光标处像素的颜色值 {}",
        hex(color_at(pointer))
    );
}

#[test]
fn magnifier_follows_pointer() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    move_to(&fake, &mut instance, Point::new(300, 200));
    let first = magnified_cell(&fake.overlay_scene().unwrap(), Point::new(300, 200), 3).unwrap();

    let pointer = Point::new(420, 330);
    move_to(&fake, &mut instance, pointer);

    let scene = fake.overlay_scene().unwrap();
    let cell = magnified_cell(&scene, pointer, 3).expect("放大镜应改为放大新位置周围的像素");
    assert_eq!(
        (cell.x - first.x, cell.y - first.y),
        (120, 130),
        "放大镜应随光标移动"
    );
    assert!(scene.text_rect(&hex(color_at(pointer))).is_some());
    assert!(
        scene
            .text_rect(&hex(color_at(Point::new(300, 200))))
            .is_none()
    );
}

#[test]
fn magnifier_is_enlarged_more_on_display_with_higher_scale() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_display(FakeDisplay::new(Rect::new(400, 0, 800, 600)).scale_factor(2.0));
    let mut instance = selecting(&fake);

    move_to(&fake, &mut instance, Point::new(100, 100));
    let normal = magnified_cell(&fake.overlay_scene().unwrap(), Point::new(100, 100), 2).unwrap();
    move_to(&fake, &mut instance, Point::new(600, 100));
    let doubled = magnified_cell(&fake.overlay_scene().unwrap(), Point::new(600, 100), 2).unwrap();

    assert_eq!(doubled.width, 2 * normal.width);
}

#[test]
fn magnifier_stays_on_display_and_off_pointer_near_display_edges() {
    let display = Rect::new(0, 0, 800, 600);
    for pointer in [
        Point::new(797, 597),
        Point::new(797, 3),
        Point::new(3, 597),
        Point::new(3, 3),
    ] {
        let fake = single_display();
        let mut instance = selecting(&fake);

        move_to(&fake, &mut instance, pointer);

        let scene = fake.overlay_scene().unwrap();
        let cell = magnified_cell(&scene, pointer, 2)
            .unwrap_or_else(|| panic!("光标在 {pointer:?} 时应显示放大镜"));
        let grid = Rect::new(
            cell.x - 2 * cell.width as i32,
            cell.y - 2 * cell.height as i32,
            5 * cell.width,
            5 * cell.height,
        );
        assert_eq!(
            display.intersection(grid),
            Some(grid),
            "放大镜应在显示器内：{pointer:?}"
        );
        assert!(!grid.contains(pointer), "放大镜不应挡住光标：{pointer:?}");
    }
}

fn press_c(fake: &FakePlatform, instance: &mut ResidentInstance, modifiers: Modifiers) {
    feed(
        instance,
        vec![fake.overlay_input(OverlayInput::KeyPressed {
            key: OverlayKey::C,
            modifiers,
        })],
    );
}

#[test]
fn pressing_c_copies_color_under_pointer_as_hex_and_keeps_selecting() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    let pointer = Point::new(300, 200);
    move_to(&fake, &mut instance, pointer);

    press_c(&fake, &mut instance, Modifiers::NONE);

    assert_eq!(
        fake.clipboard().text(),
        Some(hex(color_at(pointer)).as_str())
    );
    assert_eq!(fake.clipboard().formats(), ["Text"], "只复制颜色值");
    let scene = fake.overlay_scene().expect("复制颜色后仍在框选");
    assert!(
        scene.text_rect("已复制颜色").is_some(),
        "放大镜应提示已复制"
    );

    move_to(&fake, &mut instance, Point::new(301, 200));
    let scene = fake.overlay_scene().unwrap();
    assert!(
        scene.text_rect("已复制颜色").is_none(),
        "对准别的像素后不再提示"
    );
}

#[test]
fn ctrl_c_does_not_copy_color() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    move_to(&fake, &mut instance, Point::new(300, 200));

    press_c(
        &fake,
        &mut instance,
        Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        },
    );

    assert!(fake.clipboard().is_empty());
}

fn press(fake: &FakePlatform, at: Point) -> PlatformEvent {
    fake.overlay_input(OverlayInput::PointerPressed {
        button: PointerButton::Left,
        position: at,
        modifiers: Modifiers::NONE,
        double_click: false,
    })
}

#[test]
fn magnifier_follows_dragged_corner_and_hides_once_region_is_selected() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    feed(&mut instance, vec![press(&fake, Point::new(100, 100))]);

    move_to(&fake, &mut instance, Point::new(250, 180));
    assert!(
        magnified_cell(&fake.overlay_scene().unwrap(), Point::new(250, 180), 3).is_some(),
        "拖拽框选时放大镜对准拖动的角"
    );

    feed(
        &mut instance,
        vec![fake.overlay_input(OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: Point::new(250, 180),
            modifiers: Modifiers::NONE,
        })],
    );
    let scene = fake.overlay_scene().unwrap();
    assert!(
        magnified_cell(&scene, Point::new(250, 180), 1).is_none(),
        "选区框好后不显示放大镜"
    );
    assert!(
        scene
            .text_rect(&hex(color_at(Point::new(250, 180))))
            .is_none()
    );

    press_c(&fake, &mut instance, Modifiers::NONE);
    assert!(fake.clipboard().is_empty(), "不显示放大镜时 C 不复制颜色");
}

#[test]
fn magnifier_shows_dragged_edge_while_resizing_but_not_while_moving() {
    let fake = single_display();
    let mut instance = selecting(&fake);
    // 框出 (100, 100)–(249, 179)。
    feed(
        &mut instance,
        fake.drag(Point::new(100, 100), Point::new(249, 179)),
    );

    // 在边框外侧 2 像素处抓住右下角，拖动 (+30, +20)：右下角的像素变成 (279, 199)。
    feed(&mut instance, vec![press(&fake, Point::new(251, 181))]);
    move_to(&fake, &mut instance, Point::new(281, 201));
    let corner = Point::new(279, 199);
    assert!(
        magnified_cell(&fake.overlay_scene().unwrap(), corner, 3).is_some(),
        "放大镜应对准被拖动的角上的像素"
    );
    press_c(&fake, &mut instance, Modifiers::NONE);
    assert_eq!(
        fake.clipboard().text(),
        Some(hex(color_at(corner)).as_str())
    );
    feed(
        &mut instance,
        vec![fake.overlay_input(OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: Point::new(281, 201),
            modifiers: Modifiers::NONE,
        })],
    );

    // 在选区内按下拖动：移动选区时不显示放大镜。
    feed(&mut instance, vec![press(&fake, Point::new(180, 150))]);
    move_to(&fake, &mut instance, Point::new(190, 160));
    let scene = fake.overlay_scene().unwrap();
    assert!(magnified_cell(&scene, Point::new(190, 160), 1).is_none());
    assert!(
        scene
            .text_rect(&hex(color_at(Point::new(190, 160))))
            .is_none()
    );
}
