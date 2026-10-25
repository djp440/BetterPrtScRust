//! 区域捕获：先拍冻结帧、显示选区遮罩，用户在冻结帧上框选区域后，从冻结帧裁剪拼接出截图。
//!
//! 用户在遮罩上的操作由假平台模拟（`drag` / `click` / `press_key` / `select_region`
//! 生成平台事件），期望图像来自假平台的 `desktop_pattern`（每个虚拟桌面像素的颜色由坐标决定）。

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, Flow, HotkeyBinding, PostCaptureAction,
    ResidentInstance,
};
use bps_platform::{
    Hotkey, LocalDateTime, OverlayInput, OverlayKey, PlatformEvent, Point, PointerButton, Rect,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

fn region_capture(after: PostCaptureAction) -> Command {
    Command::Capture(CaptureRequest::new(CaptureMode::Region, after))
}

fn copy_region_capture() -> Command {
    region_capture(PostCaptureAction::CopyToClipboard)
}

fn copy_and_save_region_capture() -> Command {
    Command::Capture(
        CaptureRequest::new(CaptureMode::Region, PostCaptureAction::CopyToClipboard)
            .with_auto_save(),
    )
}

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
        .with_cursor_at(Point::new(30, 20))
}

/// 区域捕获已开始、遮罩已显示的常驻实例。
fn selecting(fake: &FakePlatform, command: Command) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance.execute(command).unwrap();
    assert!(fake.overlay_visible(), "区域捕获应先显示选区遮罩");
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 剪贴板、文件、通知都没有任何产物。
fn assert_no_product(fake: &FakePlatform) {
    assert!(fake.clipboard().is_empty(), "剪贴板不应有内容");
    assert!(fake.files().is_empty(), "不应写入文件：{:?}", fake.files());
    assert!(fake.notifications().is_empty(), "不应发出通知");
}

#[test]
fn region_selected_on_overlay_is_copied_to_clipboard() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    assert!(fake.clipboard().is_empty(), "框选完成前不应产生截图");

    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));

    assert!(!fake.overlay_visible(), "确定选区后遮罩应关闭");
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    assert!(clipboard.png_image().unwrap() == desktop_pattern(region));
    assert!(clipboard.dib_image().unwrap() == desktop_pattern(region));
}

#[test]
fn escape_cancels_region_capture_without_any_product() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_and_save_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible(), "取消后遮罩应关闭");
    assert_no_product(&fake);
}

#[test]
fn escape_before_selecting_anything_cancels_region_capture() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_and_save_region_capture());

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
}

#[test]
fn right_click_clears_selection_and_second_right_click_cancels() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    feed(
        &mut instance,
        fake.click(PointerButton::Right, Point::new(40, 40)),
    );
    assert!(fake.overlay_visible(), "第一次右键只清除选区");
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);
    assert!(
        fake.clipboard().is_empty(),
        "选区已清除，Enter 不应完成框选"
    );

    feed(
        &mut instance,
        fake.click(PointerButton::Right, Point::new(40, 40)),
    );
    assert!(!fake.overlay_visible(), "没有选区时右键取消捕获");
    assert_no_product(&fake);
}

#[test]
fn interrupted_overlay_cancels_region_capture() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    feed(
        &mut instance,
        vec![fake.overlay_input(OverlayInput::Interrupted)],
    );

    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
}

#[test]
fn region_comes_from_frozen_frame_taken_before_overlay_appeared() {
    // 按下热键时屏幕上有一个悬停菜单；遮罩出现后鼠标移开，菜单消失、画面变了。
    let with_hover_menu = desktop_pattern(Rect::new(1000, 1000, 64, 48));
    let fake = FakePlatform::new()
        .with_display(
            FakeDisplay::new(Rect::new(0, 0, 64, 48))
                .primary()
                .with_image(with_hover_menu.clone()),
        )
        .with_cursor_at(Point::new(30, 20));
    let mut instance = selecting(&fake, copy_region_capture());

    fake.change_display_image(0, desktop_pattern(Rect::new(0, 0, 64, 48)));
    feed(&mut instance, fake.select_region(Rect::new(0, 0, 64, 48)));

    assert!(
        fake.clipboard().png_image().unwrap() == with_hover_menu,
        "截图应是冻结帧（带悬停菜单）的内容，而不是之后的画面"
    );
}

#[test]
fn region_spanning_displays_with_different_scale_factors_is_stitched_in_physical_pixels() {
    // 主显示器 100%；右侧副显示器 150%，向上错位 10 像素。
    let displays = [
        FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary(),
        FakeDisplay::new(Rect::new(64, -10, 96, 54)).scale_factor(1.5),
    ];
    let fake = displays
        .iter()
        .cloned()
        .fold(FakePlatform::new(), FakePlatform::with_display)
        .with_cursor_at(Point::new(10, 10));
    let mut instance = selecting(&fake, copy_region_capture());
    // 从主显示器拖到副显示器；左上角 (50, -5) 一带没有任何显示器覆盖。
    let region = Rect::new(50, -5, 40, 30);

    feed(&mut instance, fake.select_region(region));

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (40, 30));
    let pattern = desktop_pattern(region);
    for row in 0..region.height {
        for col in 0..region.width {
            let point = Point::new(region.x + col as i32, region.y + row as i32);
            let covered = displays.iter().any(|d| d.bounds().contains(point));
            let expected = if covered {
                pattern.pixel(col, row)
            } else {
                [0, 0, 0, 0]
            };
            assert_eq!(png.pixel(col, row), expected, "像素 {point:?}");
        }
    }
}

#[test]
fn dragging_up_and_left_selects_region_including_both_end_pixels() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());

    feed(
        &mut instance,
        fake.drag(Point::new(29, 16), Point::new(10, 5)),
    );
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(Rect::new(10, 5, 20, 12)));
}

#[test]
fn click_without_dragging_does_not_select_anything() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(20, 20)),
    );
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(fake.overlay_visible(), "没有选区时 Enter 不结束捕获");
    assert!(fake.clipboard().is_empty());
}

#[test]
fn double_click_inside_selection_completes_it() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );
    assert!(
        fake.clipboard().is_empty(),
        "松开鼠标只是确定选区，还没有完成"
    );

    feed(&mut instance, fake.double_click(Point::new(15, 10)));

    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(Rect::new(10, 5, 20, 12)));
}

#[test]
fn dragging_again_outside_selection_replaces_it_and_click_outside_keeps_it() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(49, 39)),
    );
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(2, 2)),
    );
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(Rect::new(40, 30, 10, 10)));
}

#[test]
fn selection_is_limited_to_virtual_desktop() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());

    feed(
        &mut instance,
        fake.drag(Point::new(50, 40), Point::new(500, 300)),
    );
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(Rect::new(50, 40, 14, 8)));
}

#[test]
fn auto_saved_region_is_named_after_moment_frozen_frame_was_taken() {
    let fake = single_display()
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 30, 15))
        .with_pictures_dir(r"C:\Users\me\Pictures");
    let mut instance = selecting(&fake, copy_and_save_region_capture());

    // 用户框选花了一分多钟。
    fake.set_time(LocalDateTime::new(2026, 10, 24, 9, 31, 40));
    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));

    let path = std::path::Path::new(r"C:\Users\me\Pictures\BetterPrtSc\2026-10-24 09-30-15.png");
    assert_eq!(fake.files(), [path]);
    assert_eq!(fake.clipboard().files(), Some(vec![path.to_path_buf()]));
}

#[test]
fn another_capture_is_rejected_while_overlay_is_open() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());

    let display_capture = Command::Capture(CaptureRequest::new(
        CaptureMode::Display,
        PostCaptureAction::CopyToClipboard,
    ));
    assert_eq!(
        instance.execute(display_capture),
        Err(CoreError::CaptureInProgress)
    );
    assert_eq!(
        instance.execute(copy_region_capture()),
        Err(CoreError::CaptureInProgress)
    );
    assert!(fake.clipboard().is_empty());

    // 进行中的框选不受影响。
    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));
    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(region));
}

#[test]
fn exit_during_region_capture_closes_overlay_without_any_product() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_and_save_region_capture());
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    assert_eq!(instance.execute(Command::Exit), Ok(Flow::Exit));

    assert!(!fake.overlay_visible());
    assert_no_product(&fake);
}

#[test]
fn hotkey_bound_to_region_capture_opens_overlay_and_copies_selected_region() {
    let fake = single_display();
    let hotkey: Hotkey = "Ctrl+Alt+A".parse().unwrap();
    let binding = HotkeyBinding {
        hotkey,
        capture: CaptureRequest::new(CaptureMode::Region, PostCaptureAction::CopyToClipboard),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    assert!(instance.register_hotkeys().is_empty());

    instance.handle(fake.press(hotkey).unwrap()).unwrap();
    assert!(fake.overlay_visible());
    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));

    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(region));
}

#[test]
fn next_region_capture_can_start_after_previous_one_ends() {
    let fake = single_display();
    let mut instance = selecting(&fake, copy_region_capture());
    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    instance.execute(copy_region_capture()).unwrap();
    let region = Rect::new(1, 2, 3, 4);
    feed(&mut instance, fake.select_region(region));

    assert!(fake.clipboard().png_image().unwrap() == desktop_pattern(region));
}
