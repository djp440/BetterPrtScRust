//! 调整选区：选区框好后，拖动边框上的手柄改变大小、在选区内拖动移动它、用方向键逐像素微调；
//! 最终截图与调整后的选区完全一致。
//!
//! 用户在遮罩上的操作由假平台模拟；期望截图来自假平台的 `desktop_pattern`。

use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
use bps_platform::{
    Modifiers, OverlayCursor, OverlayInput, OverlayItem, OverlayKey, OverlayScene, PlatformEvent,
    Point, PointerButton, Rect,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

const DESKTOP: Rect = Rect::new(0, 0, 800, 600);

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(DESKTOP).primary())
        .with_cursor_at(Point::new(10, 10))
}

fn capture(fake: &FakePlatform, after: PostCaptureAction) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            after,
        )))
        .unwrap();
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn right(rect: Rect) -> i32 {
    rect.x + rect.width as i32
}

fn bottom(rect: Rect) -> i32 {
    rect.y + rect.height as i32
}

/// 框选 `region`（从左上角的像素拖到右下角的像素），不按 Enter。
fn select(fake: &FakePlatform, instance: &mut ResidentInstance, region: Rect) {
    feed(
        instance,
        fake.drag(
            Point::new(region.x, region.y),
            Point::new(right(region) - 1, bottom(region) - 1),
        ),
    );
}

/// 按 Enter 确定，返回复制到剪贴板的截图的范围应有的画面是否与 `expected` 一致。
fn confirm_and_assert_captured(
    fake: &FakePlatform,
    instance: &mut ResidentInstance,
    expected: Rect,
) {
    feed(instance, vec![fake.press_key(OverlayKey::Enter)]);
    assert!(!fake.overlay_visible(), "确定后遮罩应关闭");
    let image = fake.clipboard().png_image().expect("剪贴板里应有截图");
    assert_eq!(
        (image.width(), image.height()),
        (expected.width, expected.height),
        "截图尺寸应等于调整后的选区"
    );
    assert!(
        image == desktop_pattern(expected),
        "截图应是调整后选区的画面"
    );
}

fn size_label(fake: &FakePlatform) -> String {
    let scene = fake.overlay_scene().unwrap();
    scene.labels().next().expect("应有尺寸标签").text.clone()
}

#[test]
fn dragging_corner_handle_resizes_selection_and_capture_matches() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);
    select(&fake, &mut instance, region);

    // 右下角手柄在边框的右下角上。
    let handle = Point::new(right(region), bottom(region));
    feed(
        &mut instance,
        fake.drag(handle, Point::new(handle.x + 50, handle.y + 30)),
    );

    let adjusted = Rect::new(100, 80, 250, 180);
    assert_eq!(size_label(&fake), "250 × 180");
    confirm_and_assert_captured(&fake, &mut instance, adjusted);
}

#[test]
fn dragging_edge_handle_moves_only_that_edge() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);
    select(&fake, &mut instance, region);

    // 左边中点的手柄在左边框上；斜着拖也只改变左边。
    let handle = Point::new(region.x - 1, region.y + 75);
    feed(
        &mut instance,
        fake.drag(handle, Point::new(handle.x - 30, handle.y + 40)),
    );

    confirm_and_assert_captured(&fake, &mut instance, Rect::new(70, 80, 230, 150));
}

#[test]
fn dragging_handle_past_opposite_edge_flips_selection() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);
    select(&fake, &mut instance, region);

    // 把右边从 x = 299 拖到 x = 49：越过左边（x = 100），选区变成 49..=100。
    let handle = Point::new(right(region), region.y + 75);
    feed(
        &mut instance,
        fake.drag(handle, Point::new(handle.x - 250, handle.y)),
    );

    confirm_and_assert_captured(&fake, &mut instance, Rect::new(49, 80, 52, 150));
}

#[test]
fn handles_cannot_drag_selection_off_virtual_desktop() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(600, 400, 100, 100);
    select(&fake, &mut instance, region);

    let handle = Point::new(right(region), bottom(region));
    feed(&mut instance, fake.drag(handle, Point::new(5000, 5000)));

    confirm_and_assert_captured(&fake, &mut instance, Rect::new(600, 400, 200, 200));
}

#[test]
fn dragging_inside_selection_moves_it_within_virtual_desktop() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);
    select(&fake, &mut instance, region);

    feed(
        &mut instance,
        fake.drag(Point::new(200, 150), Point::new(240, 120)),
    );
    assert_eq!(size_label(&fake), "200 × 150", "移动不改变大小");
    let label = fake
        .overlay_scene()
        .unwrap()
        .labels()
        .next()
        .unwrap()
        .clone();
    assert_eq!(label.anchor.x, 140, "尺寸标签随选区移动");

    // 继续往左上拖出虚拟桌面：选区停在桌面边缘。
    feed(
        &mut instance,
        fake.drag(Point::new(240, 120), Point::new(-500, -500)),
    );
    confirm_and_assert_captured(&fake, &mut instance, Rect::new(0, 0, 200, 150));
}

#[test]
fn clicking_inside_selection_keeps_it_and_double_click_still_finishes() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);
    select(&fake, &mut instance, region);

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(200, 150)),
    );
    assert_eq!(size_label(&fake), "200 × 150");
    feed(&mut instance, fake.double_click(Point::new(200, 150)));

    assert!(!fake.overlay_visible(), "双击选区完成框选");
    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(region));
}

fn key(fake: &FakePlatform, key: OverlayKey, modifiers: Modifiers) -> PlatformEvent {
    fake.overlay_input(OverlayInput::KeyPressed { key, modifiers })
}

const CTRL: Modifiers = Modifiers {
    ctrl: true,
    ..Modifiers::NONE
};
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::NONE
};

#[test]
fn arrow_keys_nudge_selection_pixel_by_pixel() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    select(&fake, &mut instance, Rect::new(100, 80, 200, 150));

    // 方向键移动 1 像素。
    feed(
        &mut instance,
        vec![
            key(&fake, OverlayKey::Right, Modifiers::NONE),
            key(&fake, OverlayKey::Right, Modifiers::NONE),
            key(&fake, OverlayKey::Right, Modifiers::NONE),
            key(&fake, OverlayKey::Up, Modifiers::NONE),
        ],
    );
    assert_eq!(size_label(&fake), "200 × 150");
    // Ctrl：朝方向键的那条边向外扩 1 像素。
    feed(
        &mut instance,
        vec![
            key(&fake, OverlayKey::Left, CTRL),
            key(&fake, OverlayKey::Left, CTRL),
            key(&fake, OverlayKey::Down, CTRL),
        ],
    );
    assert_eq!(size_label(&fake), "202 × 151");
    // Shift：背向方向键的那条边向内缩 1 像素（被移动的边仍朝方向键所指的方向走）。
    feed(
        &mut instance,
        vec![
            key(&fake, OverlayKey::Right, SHIFT),
            key(&fake, OverlayKey::Up, SHIFT),
            key(&fake, OverlayKey::Up, SHIFT),
        ],
    );
    assert_eq!(size_label(&fake), "201 × 149");

    // 移动后 (103, 79)–(302, 228)；扩大后 (101, 79)–(302, 229)；缩小后 (102, 79)–(302, 227)。
    confirm_and_assert_captured(&fake, &mut instance, Rect::new(102, 79, 201, 149));
}

#[test]
fn nudging_stops_at_virtual_desktop_edges_and_at_one_pixel() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    // 拖动要超过拖拽阈值（4 像素）才算框选，先框 6 像素宽再收窄。
    select(&fake, &mut instance, Rect::new(0, 0, 6, 2));

    let mut keys = vec![
        key(&fake, OverlayKey::Left, Modifiers::NONE),
        key(&fake, OverlayKey::Up, CTRL),
    ];
    keys.extend((0..6).map(|_| key(&fake, OverlayKey::Left, SHIFT)));
    feed(&mut instance, keys);

    confirm_and_assert_captured(&fake, &mut instance, Rect::new(0, 0, 1, 2));
}

/// 场景里有没有包含 `point` 的小块不透明填充（手柄）。
fn handle_at(scene: &OverlayScene, point: Point) -> bool {
    scene.items.iter().any(|item| match item {
        OverlayItem::Fill { rect, color } => {
            color.a == 255 && rect.width <= 20 && rect.height <= 20 && rect.contains(point)
        }
        _ => false,
    })
}

/// 选区边框上手柄的位置：四个角和四条边的中点（边框画在选区外侧 1 像素处）。
fn handle_points(region: Rect) -> [Point; 8] {
    let (l, t, r, b) = (region.x - 1, region.y - 1, right(region), bottom(region));
    let cx = region.x + region.width as i32 / 2;
    let cy = region.y + region.height as i32 / 2;
    [
        Point::new(l, t),
        Point::new(cx, t),
        Point::new(r, t),
        Point::new(l, cy),
        Point::new(r, cy),
        Point::new(l, b),
        Point::new(cx, b),
        Point::new(r, b),
    ]
}

#[test]
fn selected_region_shows_handles_on_its_frame() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(100, 80, 200, 150);

    select(&fake, &mut instance, region);

    let scene = fake.overlay_scene().unwrap();
    for point in handle_points(region) {
        assert!(handle_at(&scene, point), "{point:?} 处应有手柄");
    }
}

fn text_rect(fake: &FakePlatform, text: &str) -> Rect {
    fake.overlay_scene()
        .unwrap()
        .text_rect(text)
        .unwrap_or_else(|| panic!("遮罩上应有“{text}”"))
}

fn shifted(rect: Rect, dx: i32, dy: i32) -> Rect {
    Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height)
}

#[test]
fn selection_handles_remain_after_annotating_and_toolbar_follows_adjustments() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    let region = Rect::new(450, 80, 250, 200);
    select(&fake, &mut instance, region);
    let copy = text_rect(&fake, "确认");

    // 工具条贴在选区右下方（选区靠右一些，免得工具条被挤到屏幕左边缘）：拖右下角手柄、按方向键后，工具条随选区挪动。
    let handle = Point::new(right(region), bottom(region));
    feed(
        &mut instance,
        fake.drag(handle, Point::new(handle.x + 40, handle.y + 30)),
    );
    assert_eq!(text_rect(&fake, "确认"), shifted(copy, 40, 30));
    feed(
        &mut instance,
        vec![key(&fake, OverlayKey::Right, Modifiers::NONE)],
    );
    assert_eq!(text_rect(&fake, "确认"), shifted(copy, 41, 30));
    let adjusted = Rect::new(451, 80, 290, 230);
    assert!(handle_at(
        &fake.overlay_scene().unwrap(),
        handle_points(adjusted)[7]
    ));

    // 开始标注后手柄仍在；方向键不再移动区域。
    feed(&mut instance, fake.click_text("矩形"));
    let scene = fake.overlay_scene().unwrap();
    for point in handle_points(adjusted) {
        assert!(handle_at(&scene, point), "开始标注后仍有手柄：{point:?}");
    }
    feed(
        &mut instance,
        vec![key(&fake, OverlayKey::Right, Modifiers::NONE)],
    );
    assert_eq!(text_rect(&fake, "确认"), shifted(copy, 41, 30));

    feed(&mut instance, fake.click_text("确认"));
    let image = fake.clipboard().png_image().expect("剪贴板里应有截图");
    assert!(image == desktop_pattern(adjusted), "截图应是调整后的区域");
}

#[test]
fn pointer_shape_tells_what_dragging_there_does() {
    let fake = single_display();
    let mut instance = capture(&fake, PostCaptureAction::AwaitConfirmation);
    select(&fake, &mut instance, Rect::new(100, 80, 200, 150));

    let cases = [
        (Point::new(99, 79), OverlayCursor::ResizeNwse),
        (Point::new(300, 230), OverlayCursor::ResizeNwse),
        (Point::new(300, 79), OverlayCursor::ResizeNesw),
        (Point::new(99, 230), OverlayCursor::ResizeNesw),
        (Point::new(99, 150), OverlayCursor::ResizeEw),
        (Point::new(300, 150), OverlayCursor::ResizeEw),
        (Point::new(200, 79), OverlayCursor::ResizeNs),
        (Point::new(200, 230), OverlayCursor::ResizeNs),
        (Point::new(200, 150), OverlayCursor::Move),
        (Point::new(500, 400), OverlayCursor::Crosshair),
    ];
    for (point, expected) in cases {
        feed(
            &mut instance,
            vec![fake.overlay_input(OverlayInput::PointerMoved {
                position: point,
                modifiers: Modifiers::NONE,
            })],
        );
        assert_eq!(fake.overlay_scene().unwrap().cursor, expected, "{point:?}");
    }
}
