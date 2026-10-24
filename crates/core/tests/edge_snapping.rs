//! 框选区域时的边缘吸附：选区边缘进入吸附距离后对齐到窗口或控件的边缘，按住 Ctrl 时临时不吸附。
//!
//! 窗口布局来自假平台的窗口（`FakePlatform::with_window`），断言的是遮罩上画出的选区边框，
//! 即用户看到的选区。

use bps_core::{
    CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance, default_bindings,
};
use bps_platform::{
    Hotkey, Key, Modifiers, OverlayInput, OverlayItem, OverlayKey, PlatformEvent, Point,
    PointerButton, Rect,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeWindow, desktop_pattern};

/// 400 × 300 的 100% 主显示器，中间有一个窗口。
fn desktop_with_window() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_window(FakeWindow::new("记事本", WINDOW))
        .with_cursor_at(Point::new(10, 10))
}

const WINDOW: Rect = Rect::new(100, 50, 200, 100);

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

fn press(fake: &FakePlatform, at: Point, modifiers: Modifiers) -> PlatformEvent {
    fake.overlay_input(OverlayInput::PointerPressed {
        button: PointerButton::Left,
        position: at,
        modifiers,
        double_click: false,
    })
}

/// 指针移到 `at`；修饰键按下或松开而指针不动时，平台也在原位置递交一次移动。
fn move_to(fake: &FakePlatform, at: Point, modifiers: Modifiers) -> PlatformEvent {
    fake.overlay_input(OverlayInput::PointerMoved {
        position: at,
        modifiers,
    })
}

fn release(fake: &FakePlatform, at: Point, modifiers: Modifiers) -> PlatformEvent {
    fake.overlay_input(OverlayInput::PointerReleased {
        button: PointerButton::Left,
        position: at,
        modifiers,
    })
}

/// 遮罩上画出的选区（选区边框围住的矩形）。
fn selection(fake: &FakePlatform) -> Option<Rect> {
    let scene = fake.overlay_scene().expect("选区遮罩没有显示");
    scene.items.iter().find_map(|item| match item {
        OverlayItem::Frame { rect, .. } => Some(*rect),
        _ => None,
    })
}

#[test]
fn selection_snaps_to_window_edges_within_snap_distance() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 窗口占 x 100..=299、y 50..=149；从左上角外 3 像素拖到右下角内 4 像素。
    feed(
        &mut instance,
        fake.drag(Point::new(97, 47), Point::new(295, 145)),
    );

    assert_eq!(selection(&fake), Some(WINDOW));
}

#[test]
fn edges_farther_than_snap_distance_do_not_snap() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 选区四条边离窗口的边各 9、9、10、10 像素，超出 100% 下 8 像素的吸附距离。
    feed(
        &mut instance,
        fake.drag(Point::new(91, 41), Point::new(309, 159)),
    );

    assert_eq!(selection(&fake), Some(Rect::new(91, 41, 219, 119)));
}

#[test]
fn edges_exactly_at_snap_distance_snap() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 选区四条边离窗口的边都是 8 像素：左 92、上 42、右 308、下 158。
    feed(
        &mut instance,
        fake.drag(Point::new(92, 42), Point::new(307, 157)),
    );

    assert_eq!(selection(&fake), Some(WINDOW));
}

#[test]
fn snap_distance_scales_with_display_under_the_dragged_corner() {
    let window = Rect::new(200, 100, 400, 200);
    let fake = FakePlatform::new()
        .with_display(
            FakeDisplay::new(Rect::new(0, 0, 800, 600))
                .primary()
                .scale_factor(2.0),
        )
        .with_window(FakeWindow::new("记事本", window))
        .with_cursor_at(Point::new(10, 10));
    let mut instance = selecting(&fake);

    // 200% 下吸附距离是 16 像素：左上角离窗口 16 像素会吸附，右下角离窗口 17 像素不吸附。
    feed(
        &mut instance,
        fake.drag(Point::new(184, 84), Point::new(616, 316)),
    );

    assert_eq!(selection(&fake), Some(Rect::new(200, 100, 417, 217)));
}

#[test]
fn holding_ctrl_while_dragging_keeps_the_dragged_corner_under_the_pointer() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);
    let end = Point::new(295, 145);

    feed(
        &mut instance,
        vec![
            press(&fake, Point::new(97, 47), Modifiers::NONE),
            move_to(&fake, end, Modifiers::CTRL),
        ],
    );
    // 起点按下时没按 Ctrl，仍吸附在窗口左上角；拖动的一角不吸附。
    assert_eq!(selection(&fake), Some(Rect::new(100, 50, 196, 96)));

    // 松开 Ctrl（指针没动）立刻恢复吸附。
    feed(&mut instance, vec![move_to(&fake, end, Modifiers::NONE)]);
    assert_eq!(selection(&fake), Some(WINDOW));

    // 按住 Ctrl 松开左键：确定下来的选区也不吸附。
    feed(
        &mut instance,
        vec![
            move_to(&fake, end, Modifiers::CTRL),
            release(&fake, end, Modifiers::CTRL),
        ],
    );
    assert_eq!(selection(&fake), Some(Rect::new(100, 50, 196, 96)));
}

#[test]
fn pressing_with_ctrl_keeps_the_start_corner_where_it_was_pressed() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    feed(
        &mut instance,
        vec![
            press(&fake, Point::new(97, 47), Modifiers::CTRL),
            move_to(&fake, Point::new(295, 145), Modifiers::NONE),
        ],
    );

    assert_eq!(selection(&fake), Some(Rect::new(97, 47, 203, 103)));
}

#[test]
fn edges_away_from_the_dragged_corner_do_not_snap() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 起点的 x 离窗口左边只有 3 像素，但起点在窗口下方 50 像素处，那里看不到窗口左边。
    feed(
        &mut instance,
        fake.drag(Point::new(97, 200), Point::new(250, 260)),
    );

    assert_eq!(selection(&fake), Some(Rect::new(97, 200, 154, 61)));
}

#[test]
fn edges_hidden_under_other_windows_do_not_snap() {
    // 上层窗口盖住了记事本右边的上半段（y 50..120），下半段仍然看得见。
    let fake = desktop_with_window()
        .with_window(FakeWindow::new("资源管理器", Rect::new(250, 0, 150, 120)));
    let mut instance = selecting(&fake);

    feed(
        &mut instance,
        vec![
            press(&fake, Point::new(150, 70), Modifiers::NONE),
            move_to(&fake, Point::new(296, 100), Modifiers::NONE),
        ],
    );
    assert_eq!(selection(&fake), Some(Rect::new(150, 70, 147, 31)));

    feed(
        &mut instance,
        vec![move_to(&fake, Point::new(296, 140), Modifiers::NONE)],
    );
    assert_eq!(selection(&fake), Some(Rect::new(150, 70, 150, 71)));
}

#[test]
fn selection_snaps_to_control_edges_inside_the_window_under_the_pointer() {
    let control = Rect::new(130, 80, 60, 40);
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_window(FakeWindow::new("记事本", WINDOW).with_control(control))
        .with_cursor_at(Point::new(10, 10));
    let mut instance = selecting(&fake);

    // 两角都离控件的角 3 像素，离窗口的边 27 像素以上。
    feed(
        &mut instance,
        fake.drag(Point::new(127, 77), Point::new(186, 116)),
    );

    assert_eq!(selection(&fake), Some(control));
}

#[test]
fn screenshot_is_the_snapped_selection() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    let mut events = fake.drag(Point::new(97, 47), Point::new(295, 145));
    events.push(fake.press_key(OverlayKey::Enter));
    feed(&mut instance, events);

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(WINDOW));
}

#[test]
fn dragging_up_and_left_snaps_the_same_edges() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 从右下角外 3 像素拖到左上角内 3 像素。
    feed(
        &mut instance,
        fake.drag(Point::new(302, 152), Point::new(103, 53)),
    );

    assert_eq!(selection(&fake), Some(WINDOW));
}

#[test]
fn selection_beside_a_window_ends_right_where_the_window_begins() {
    let fake = desktop_with_window();
    let mut instance = selecting(&fake);

    // 在窗口左侧框选，右边停在离窗口左边 4 像素处：选区最右一列紧挨着窗口最左一列（x = 100）。
    feed(
        &mut instance,
        fake.drag(Point::new(20, 70), Point::new(95, 100)),
    );

    assert_eq!(selection(&fake), Some(Rect::new(20, 70, 80, 31)));
}

#[test]
fn selection_never_snaps_past_the_virtual_desktop() {
    // 窗口右边伸出桌面 4 像素（x = 404），桌面右边是 400。
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_window(FakeWindow::new("记事本", Rect::new(350, 100, 54, 80)))
        .with_cursor_at(Point::new(10, 10));
    let mut instance = selecting(&fake);

    feed(
        &mut instance,
        fake.drag(Point::new(360, 110), Point::new(420, 170)),
    );

    assert_eq!(selection(&fake), Some(Rect::new(360, 110, 40, 61)));
}

#[test]
fn framing_a_region_to_record_snaps_too() {
    let fake = desktop_with_window();
    let mut instance = ResidentInstance::new(fake.platform(), default_bindings());
    assert!(instance.register_hotkeys().is_empty());
    instance
        .handle(
            fake.press(Hotkey::new(Modifiers::ALT, Key::PrintScreen))
                .unwrap(),
        )
        .unwrap();

    feed(
        &mut instance,
        vec![
            press(&fake, Point::new(97, 47), Modifiers::NONE),
            move_to(&fake, Point::new(295, 145), Modifiers::NONE),
        ],
    );

    assert_eq!(selection(&fake), Some(WINDOW));
}
