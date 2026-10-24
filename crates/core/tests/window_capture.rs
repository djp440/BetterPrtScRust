//! 窗口捕获：先拍冻结帧、显示选区遮罩，悬停时高亮光标下的顶层窗口，单击后截取这个窗口的
//! 完整内容（被遮挡的部分也在内）。
//!
//! 桌面上的窗口由假平台描述（`FakeWindow`，后添加的在上层）；窗口截图来自假平台为每个窗口
//! 准备的画面，与冻结帧（`desktop_pattern`）不同，所以能区分“截的是窗口”还是“从冻结帧裁剪”。

use std::path::Path;

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, FilenameTemplate, Flow, OutputFormat,
    PostCaptureAction, ResidentInstance,
};
use bps_platform::{
    Hotkey, Image, Key, Modifiers, OverlayInput, OverlayItem, OverlayKey, OverlayScene,
    PlatformEvent, Point, PointerButton, Rect,
};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, FakeWindow, decode};

fn window_capture(actions: Vec<PostCaptureAction>) -> Command {
    Command::Capture(CaptureRequest::new(CaptureMode::Window, actions))
}

fn copy_window_capture() -> Command {
    window_capture(vec![PostCaptureAction::CopyToClipboard])
}

const DESKTOP: Rect = Rect::new(0, 0, 200, 150);

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 200, 150)).primary())
        .with_cursor_at(Point::new(190, 140))
}

/// 窗口捕获已开始、遮罩已显示的常驻实例。
fn picking(fake: &FakePlatform, command: Command) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance.execute(command).unwrap();
    assert!(fake.overlay_visible(), "窗口捕获应先显示选区遮罩");
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn move_pointer(fake: &FakePlatform, to: Point) -> Vec<PlatformEvent> {
    vec![fake.overlay_input(OverlayInput::PointerMoved {
        position: to,
        modifiers: Modifiers::NONE,
    })]
}

/// 遮罩上没有变暗的部分：虚拟桌面里不被任何遮挡覆盖的矩形。断言它恰好是 `expected`。
fn assert_only_undimmed(scene: &OverlayScene, expected: Rect) {
    let dimmed: Vec<Rect> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            OverlayItem::Fill { rect, color } if color.a > 0 => Some(*rect),
            _ => None,
        })
        .collect();
    assert!(
        dimmed
            .iter()
            .all(|rect| rect.intersection(expected).is_none()),
        "{expected:?} 内不应变暗：{dimmed:?}"
    );
    let area = |r: &Rect| u64::from(r.width) * u64::from(r.height);
    assert_eq!(
        dimmed.iter().map(area).sum::<u64>(),
        area(&DESKTOP) - area(&expected),
        "{expected:?} 以外应全部变暗且不重叠：{dimmed:?}"
    );
}

/// 整个虚拟桌面都变暗，没有高亮任何窗口。
fn assert_nothing_highlighted(scene: &OverlayScene) {
    let dimmed: Vec<Rect> = scene
        .items
        .iter()
        .filter_map(|item| match item {
            OverlayItem::Fill { rect, color } if color.a > 0 => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(dimmed, [DESKTOP]);
}

/// 剪贴板、文件、通知都没有任何产物。
fn assert_no_product(fake: &FakePlatform) {
    assert!(fake.clipboard().is_empty(), "剪贴板不应有内容");
    assert!(fake.files().is_empty(), "不应写入文件：{:?}", fake.files());
    assert!(fake.notifications().is_empty(), "不应发出通知");
}

fn copy_and_save_window_capture() -> Command {
    window_capture(vec![
        PostCaptureAction::CopyToClipboard,
        PostCaptureAction::AutoSave,
    ])
}

#[test]
fn clicking_window_on_overlay_copies_its_content_to_clipboard() {
    let editor = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60));
    let fake = desktop().with_window(editor.clone());
    let mut instance = picking(&fake, copy_window_capture());
    assert!(fake.clipboard().is_empty(), "单击窗口之前不应产生截图");

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );

    assert!(!fake.overlay_visible(), "选中窗口后遮罩应关闭");
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    assert!(clipboard.png_image().unwrap() == *editor.image());
}

#[test]
fn escape_cancels_window_capture_without_any_product() {
    let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(20, 10, 80, 60)));
    let mut instance = picking(&fake, copy_and_save_window_capture());
    feed(&mut instance, move_pointer(&fake, Point::new(50, 40)));

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible(), "取消后遮罩应关闭");
    assert_no_product(&fake);
}

#[test]
fn right_click_or_interruption_cancels_window_capture() {
    for cancel in [
        |fake: &FakePlatform| fake.click(PointerButton::Right, Point::new(50, 40)),
        |fake: &FakePlatform| vec![fake.overlay_input(OverlayInput::Interrupted)],
    ] {
        let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(20, 10, 80, 60)));
        let mut instance = picking(&fake, copy_and_save_window_capture());

        feed(&mut instance, cancel(&fake));

        assert!(!fake.overlay_visible(), "取消后遮罩应关闭");
        assert_no_product(&fake);
    }
}

#[test]
fn hovered_window_is_highlighted_and_rest_of_desktop_dimmed() {
    let window = Rect::new(20, 10, 80, 60);
    let fake = desktop().with_window(FakeWindow::new("记事本", window));
    let mut instance = picking(&fake, copy_window_capture());
    assert_nothing_highlighted(&fake.overlay_scene().unwrap());

    feed(&mut instance, move_pointer(&fake, Point::new(50, 40)));
    assert_only_undimmed(&fake.overlay_scene().unwrap(), window);

    feed(&mut instance, move_pointer(&fake, Point::new(150, 120)));
    assert_nothing_highlighted(&fake.overlay_scene().unwrap());
}

#[test]
fn topmost_window_under_pointer_is_highlighted_where_windows_overlap() {
    let below = Rect::new(20, 10, 80, 60);
    let above = Rect::new(60, 40, 100, 80);
    let fake = desktop()
        .with_window(FakeWindow::new("下层", below))
        .with_window(FakeWindow::new("上层", above));
    let mut instance = picking(&fake, copy_window_capture());

    feed(&mut instance, move_pointer(&fake, Point::new(80, 60)));
    assert_only_undimmed(&fake.overlay_scene().unwrap(), above);

    feed(&mut instance, move_pointer(&fake, Point::new(30, 20)));
    assert_only_undimmed(&fake.overlay_scene().unwrap(), below);
}

#[test]
fn window_under_cursor_is_highlighted_as_soon_as_overlay_appears() {
    let window = Rect::new(20, 10, 80, 60);
    let fake = desktop()
        .with_window(FakeWindow::new("记事本", window))
        .with_cursor_at(Point::new(50, 40));

    let _instance = picking(&fake, copy_window_capture());

    assert_only_undimmed(&fake.overlay_scene().unwrap(), window);
}

#[test]
fn highlight_of_window_partly_off_screen_is_clipped_to_virtual_desktop() {
    let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(150, 100, 120, 90)));
    let mut instance = picking(&fake, copy_window_capture());

    feed(&mut instance, move_pointer(&fake, Point::new(160, 110)));

    assert_only_undimmed(&fake.overlay_scene().unwrap(), Rect::new(150, 100, 50, 50));
}

#[test]
fn window_partly_covered_by_another_window_is_captured_completely() {
    let covered = FakeWindow::new("被遮挡的窗口", Rect::new(20, 10, 80, 60));
    let fake = desktop()
        .with_window(covered.clone())
        .with_window(FakeWindow::new("上层", Rect::new(60, 40, 100, 80)));
    let mut instance = picking(&fake, copy_window_capture());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(30, 20)),
    );

    let captured = fake.clipboard().png_image().unwrap();
    assert!(
        captured == *covered.image(),
        "截图应是窗口自己的完整内容，而不是冻结帧上看到的样子"
    );
}

#[test]
fn clicking_on_control_inside_window_captures_whole_top_level_window() {
    let control = Rect::new(30, 30, 40, 20);
    let window = FakeWindow::new("对话框", Rect::new(20, 10, 80, 60)).with_control(control);
    let fake = desktop().with_window(window.clone());
    let mut instance = picking(&fake, copy_window_capture());

    feed(&mut instance, move_pointer(&fake, Point::new(40, 35)));
    assert_only_undimmed(&fake.overlay_scene().unwrap(), window.bounds());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(40, 35)),
    );
    assert!(fake.clipboard().png_image().unwrap() == *window.image());
}

#[test]
fn clicking_where_there_is_no_window_keeps_overlay_open() {
    let window = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60));
    let fake = desktop().with_window(window.clone());
    let mut instance = picking(&fake, copy_window_capture());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(150, 120)),
    );
    assert!(fake.overlay_visible(), "桌面空白处单击不应结束捕获");
    assert!(fake.clipboard().is_empty());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );
    assert!(fake.clipboard().png_image().unwrap() == *window.image());
}

#[test]
fn pressing_on_one_window_and_releasing_on_another_selects_nothing() {
    let fake = desktop()
        .with_window(FakeWindow::new("左", Rect::new(0, 0, 100, 150)))
        .with_window(FakeWindow::new("右", Rect::new(100, 0, 100, 150)));
    let mut instance = picking(&fake, copy_window_capture());

    feed(
        &mut instance,
        fake.drag(Point::new(50, 50), Point::new(150, 50)),
    );

    assert!(fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
}

#[test]
fn enter_captures_highlighted_window() {
    let window = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60));
    let fake = desktop().with_window(window.clone());
    let mut instance = picking(&fake, copy_window_capture());

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);
    assert!(fake.overlay_visible(), "没有高亮的窗口时 Enter 什么也不做");

    feed(&mut instance, move_pointer(&fake, Point::new(50, 40)));
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);
    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().png_image().unwrap() == *window.image());
}

#[test]
fn window_title_fills_filename_template_with_illegal_characters_replaced() {
    let fake = desktop().with_window(FakeWindow::new(
        "报告: 第1版 / 草稿",
        Rect::new(20, 10, 80, 60),
    ));
    let mut request = CaptureRequest::new(CaptureMode::Window, vec![PostCaptureAction::AutoSave]);
    request.output.filename_template = FilenameTemplate::parse("{window_title} {mode}").unwrap();
    let mut instance = picking(&fake, Command::Capture(request));

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );

    let expected = Path::new(DEFAULT_PICTURES_DIR)
        .join("BetterPrtSc")
        .join("报告_ 第1版 _ 草稿 window.png");
    assert_eq!(fake.files(), [expected]);
}

/// 四个角各有一个圆角以外的透明像素、其余是不透明灰色的窗口内容。
fn rounded_window_image(width: u32, height: u32) -> Image {
    let mut pixels = [0x80, 0x80, 0x80, 255].repeat(width as usize * height as usize);
    for (x, y) in [
        (0, 0),
        (width - 1, 0),
        (0, height - 1),
        (width - 1, height - 1),
    ] {
        let i = (y as usize * width as usize + x as usize) * 4;
        pixels[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
    }
    Image::from_rgba(width, height, pixels)
}

#[test]
fn pixels_outside_rounded_corners_stay_transparent_in_png() {
    let window = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60))
        .with_image(rounded_window_image(80, 60));
    let fake = desktop().with_window(window);
    let mut instance = picking(&fake, copy_window_capture());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!(png.pixel(0, 0)[3], 0, "圆角以外应透明");
    assert_eq!(png.pixel(79, 59)[3], 0, "圆角以外应透明");
    assert_eq!(png.pixel(40, 30), [0x80, 0x80, 0x80, 255]);
}

#[test]
fn pixels_outside_rounded_corners_become_white_in_jpg() {
    let window = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60))
        .with_image(rounded_window_image(80, 60));
    let fake = desktop().with_window(window);
    let mut request = CaptureRequest::new(CaptureMode::Window, vec![PostCaptureAction::AutoSave]);
    request.output.output_format = OutputFormat::Jpeg;
    let mut instance = picking(&fake, Command::Capture(request));

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );

    let saved = decode::jpeg(&fake.file(&fake.files()[0]).unwrap());
    let near =
        |actual: [u8; 4], expected: u8| actual[..3].iter().all(|c| c.abs_diff(expected) <= 24);
    assert!(
        near(saved.pixel(79, 0), 255),
        "圆角以外应是白色：{:?}",
        saved.pixel(79, 0)
    );
    assert!(near(saved.pixel(40, 30), 0x80), "{:?}", saved.pixel(40, 30));
}

#[test]
fn ctrl_print_screen_default_binding_copies_and_saves_clicked_window() {
    let window = FakeWindow::new("记事本", Rect::new(20, 10, 80, 60));
    let fake = desktop().with_window(window.clone());
    let mut instance = ResidentInstance::start(fake.platform());

    let pressed = fake
        .press(Hotkey::new(Modifiers::CTRL, Key::PrintScreen))
        .expect("默认绑定应注册 Ctrl+PrtSc");
    assert_eq!(instance.handle(pressed), Ok(Flow::Continue));
    assert!(fake.overlay_visible(), "Ctrl+PrtSc 应显示选区遮罩");
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(50, 40)),
    );

    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB", "Files"]);
    assert!(clipboard.png_image().unwrap() == *window.image());
    let saved = fake.files();
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert!(decode::png(&fake.file(&saved[0]).unwrap()) == *window.image());
    assert_eq!(clipboard.files().unwrap(), saved);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "截图已保存");
}

#[test]
fn window_closed_before_click_is_reported_as_capture_failure() {
    let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(20, 10, 80, 60)));
    let mut instance = ResidentInstance::start(fake.platform());
    let pressed = fake
        .press(Hotkey::new(Modifiers::CTRL, Key::PrintScreen))
        .unwrap();
    instance.handle(pressed).unwrap();

    fake.close_window(0);
    let mut results = Vec::new();
    for event in fake.click(PointerButton::Left, Point::new(50, 40)) {
        results.push(instance.handle(event));
    }

    assert!(
        matches!(results.last(), Some(Err(CoreError::Platform(_)))),
        "{results:?}"
    );
    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
    assert!(fake.files().is_empty());
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "捕获失败");
}

#[test]
fn another_capture_is_rejected_while_choosing_window() {
    let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(20, 10, 80, 60)));
    let mut instance = picking(&fake, copy_window_capture());

    let result = instance.execute(Command::Capture(CaptureRequest::new(
        CaptureMode::Display,
        vec![PostCaptureAction::CopyToClipboard],
    )));

    assert_eq!(result, Err(CoreError::CaptureInProgress));
    assert!(fake.overlay_visible(), "进行中的窗口选择不受影响");
    assert!(fake.clipboard().is_empty());
}

#[test]
fn exit_while_choosing_window_closes_overlay() {
    let fake = desktop().with_window(FakeWindow::new("记事本", Rect::new(20, 10, 80, 60)));
    let mut instance = picking(&fake, copy_window_capture());

    assert_eq!(instance.execute(Command::Exit), Ok(Flow::Exit));

    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
}
