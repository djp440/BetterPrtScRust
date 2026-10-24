//! 标注：截后动作包含标注时，选区旁浮现标注工具条；用户画框、箭头，最后选择复制到剪贴板、
//! 自动存储或另存为结束这次捕获。
//!
//! 用户在遮罩上的操作由假平台模拟：工具条按钮用 `click_text`（点击遮罩上写着这段文字的地方），
//! 画标注用 `drag`。期望图像来自假平台的 `desktop_pattern`。

use std::path::{Path, PathBuf};

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, OutputFormat, PostCaptureAction,
    ResidentInstance,
};
use bps_platform::{
    FileType, LocalDateTime, NotificationClick, PlatformEvent, Point, Rect, SaveAsOptions,
};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, decode, desktop_pattern};

/// 工具条上的全部按钮文字。
const TOOLBAR: [&str; 6] = ["矩形", "箭头", "取消", "另存为", "自动存储", "复制"];

fn annotate(mode: CaptureMode) -> Command {
    Command::Capture(CaptureRequest::new(mode, vec![PostCaptureAction::Annotate]))
}

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_cursor_at(Point::new(30, 20))
}

fn start(fake: &FakePlatform, command: Command) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance.execute(command).unwrap();
    assert!(fake.overlay_visible(), "应先显示选区遮罩");
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

/// 从 `region` 左上角的像素拖到右下角的像素（不按 Enter）。
fn drag_region(fake: &FakePlatform, region: Rect) -> Vec<PlatformEvent> {
    fake.drag(Point::new(region.x, region.y), bottom_right(region))
}

fn bottom_right(rect: Rect) -> Point {
    Point::new(
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
    )
}

fn toolbar_rects(fake: &FakePlatform) -> Vec<Rect> {
    let scene = fake.overlay_scene().expect("遮罩应显示着");
    TOOLBAR
        .iter()
        .map(|text| {
            scene
                .text_rect(text)
                .unwrap_or_else(|| panic!("工具条上应有“{text}”"))
        })
        .collect()
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.intersection(inner) == Some(inner)
}

#[test]
fn toolbar_appears_right_below_selected_region_when_capture_annotates() {
    let fake = single_display();
    let mut instance = start(&fake, annotate(CaptureMode::Region));
    let region = Rect::new(100, 80, 300, 200);

    feed(&mut instance, drag_region(&fake, region));

    let buttons = toolbar_rects(&fake);
    let toolbar = buttons.iter().copied().reduce(|a, b| a.union(b)).unwrap();
    let region_bottom = region.y + region.height as i32;
    assert!(
        toolbar.y >= region_bottom && toolbar.y <= region_bottom + 20,
        "工具条应紧贴在选区下方：{toolbar:?}"
    );
    let region_right = region.x + region.width as i32;
    assert!(
        (toolbar.x + toolbar.width as i32 - region_right).abs() <= 10,
        "工具条应与选区右边对齐：{toolbar:?}"
    );
    assert!(fake.clipboard().is_empty(), "选择结束方式之前不应产生截图");
}

#[test]
fn toolbar_is_hidden_while_dragging_and_absent_without_annotate() {
    let fake = single_display();
    let mut instance = start(&fake, annotate(CaptureMode::Region));
    let mut drag = fake.drag(Point::new(100, 80), Point::new(399, 279));
    drag.pop(); // 还没松开左键
    feed(&mut instance, drag);
    assert!(
        fake.overlay_scene().unwrap().text_rect("复制").is_none(),
        "拖拽框选期间不显示工具条"
    );

    let fake = single_display();
    let mut instance = start(
        &fake,
        Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            vec![PostCaptureAction::CopyToClipboard],
        )),
    );
    feed(
        &mut instance,
        drag_region(&fake, Rect::new(100, 80, 300, 200)),
    );
    assert!(
        fake.overlay_scene().unwrap().text_rect("复制").is_none(),
        "不含标注的捕获没有工具条"
    );
}

#[test]
fn copy_button_without_annotations_copies_the_region() {
    let fake = single_display();
    let mut instance = start(&fake, annotate(CaptureMode::Region));
    let region = Rect::new(100, 80, 300, 200);
    feed(&mut instance, drag_region(&fake, region));

    feed(&mut instance, fake.click_text("复制"));

    assert!(!fake.overlay_visible(), "选择结束方式后遮罩应关闭");
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB"]);
    assert!(clipboard.png_image().unwrap() == desktop_pattern(region));
    assert!(fake.files().is_empty());
}

#[test]
fn toolbar_stays_within_the_display_of_the_region() {
    // 左边主显示器 800x600；右边副显示器 200%，比主显示器高。
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_display(FakeDisplay::new(Rect::new(800, -200, 1600, 1200)).scale_factor(2.0))
        .with_cursor_at(Point::new(30, 20));
    let primary = Rect::new(0, 0, 800, 600);
    let cases = [
        // 选区贴着主显示器底边：下方放不下。
        Rect::new(300, 400, 300, 200),
        // 选区占满整个主显示器：上下都放不下。
        primary,
        // 选区很窄、贴着主显示器左下角：右对齐会伸出左边。
        Rect::new(0, 560, 30, 40),
        // 选区贴着主显示器右下角：工具条不能跑到右边的副显示器上。
        Rect::new(700, 500, 100, 100),
    ];
    for region in cases {
        let mut instance = start(&fake, annotate(CaptureMode::Region));
        feed(&mut instance, drag_region(&fake, region));

        for button in toolbar_rects(&fake) {
            assert!(
                contains(primary, button),
                "选区 {region:?}：按钮 {button:?} 超出了所在显示器"
            );
        }
        feed(&mut instance, fake.click_text("取消"));
        assert!(!fake.overlay_visible(), "“取消”结束这次捕获");
    }
    assert!(fake.clipboard().is_empty());
}

/// 默认标注颜色：红色。
fn is_red(pixel: [u8; 4]) -> bool {
    let [r, g, b, a] = pixel;
    r >= 200 && g <= 80 && b <= 80 && a == 255
}

/// 已框好 `region` 的标注捕获。
fn annotating(fake: &FakePlatform, region: Rect) -> ResidentInstance {
    let mut instance = start(fake, annotate(CaptureMode::Region));
    feed(&mut instance, drag_region(fake, region));
    instance
}

#[test]
fn rectangle_drawn_on_region_is_in_copied_screenshot() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);

    feed(&mut instance, fake.click_text("矩形"));
    // 在区域里从 (150,120) 拖到 (249,179)：截图坐标 (50,40)–(149,99)。
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );
    feed(&mut instance, fake.click_text("复制"));

    let png = fake.clipboard().png_image().expect("应复制到剪贴板");
    let base = desktop_pattern(region);
    assert_eq!((png.width(), png.height()), (300, 200));
    for (x, y) in [
        (50, 40),
        (149, 99),
        (100, 40),
        (50, 70),
        (149, 70),
        (100, 99),
    ] {
        assert!(
            is_red(png.pixel(x, y)),
            "框的线条 ({x},{y})：{:?}",
            png.pixel(x, y)
        );
    }
    for (x, y) in [(100, 70), (49, 39), (150, 100), (10, 10), (290, 190)] {
        assert_eq!(png.pixel(x, y), base.pixel(x, y), "框以外 ({x},{y}) 不变");
    }
    assert!(!fake.overlay_visible());
}

#[test]
fn arrow_drawn_on_region_is_in_copied_screenshot() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);

    feed(&mut instance, fake.click_text("箭头"));
    // 截图坐标 (20,100) → (220,100) 的水平箭头。
    feed(
        &mut instance,
        fake.drag(Point::new(120, 180), Point::new(320, 180)),
    );
    feed(&mut instance, fake.click_text("复制"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    for x in [25, 100, 200, 215] {
        assert!(is_red(png.pixel(x, 100)), "箭头上 ({x},100)");
    }
    for (x, y) in [(100, 80), (100, 120), (10, 100), (240, 100)] {
        assert_eq!(png.pixel(x, y), base.pixel(x, y), "箭头以外 ({x},{y})");
    }
}

#[test]
fn overlay_shows_exactly_the_screenshot_that_will_be_produced() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );
    feed(&mut instance, fake.click_text("箭头"));
    // 箭头与框交叉，再画一个之后又拖着改动的箭头。
    feed(
        &mut instance,
        fake.drag(Point::new(110, 250), Point::new(380, 100)),
    );

    let (area, shown) = fake.overlay_canvas().expect("标注时遮罩上应有画布");
    assert_eq!(area, region, "画布正好盖住区域");
    feed(&mut instance, fake.click_text("复制"));

    let png = fake.clipboard().png_image().unwrap();
    assert!(png == shown, "屏幕上看到的应与最终截图逐像素一致");
}

#[test]
fn canvas_follows_the_annotation_while_it_is_being_dragged() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    let mut drag = fake.drag(Point::new(150, 120), Point::new(249, 179));
    let release = drag.pop().unwrap();
    feed(&mut instance, drag);

    let (_, dragging) = fake.overlay_canvas().unwrap();
    assert!(is_red(dragging.pixel(149, 99)), "拖拽中的框已经显示出来");
    assert!(is_red(dragging.pixel(50, 40)));

    feed(&mut instance, vec![release]);
    let (_, released) = fake.overlay_canvas().unwrap();
    assert!(released == dragging, "松开时位置没变，画面不变");
}

#[test]
fn enter_copies_the_annotated_screenshot() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );

    feed(
        &mut instance,
        vec![fake.press_key(bps_platform::OverlayKey::Enter)],
    );

    let png = fake.clipboard().png_image().unwrap();
    assert!(is_red(png.pixel(50, 40)));
    assert!(!fake.overlay_visible());
}

#[test]
fn escape_cancels_annotation_without_any_product() {
    let fake = single_display();
    let mut instance = annotating(&fake, Rect::new(100, 80, 300, 200));
    feed(&mut instance, fake.click_text("箭头"));
    feed(
        &mut instance,
        fake.drag(Point::new(120, 180), Point::new(320, 180)),
    );

    feed(
        &mut instance,
        vec![fake.press_key(bps_platform::OverlayKey::Escape)],
    );

    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
    assert!(fake.files().is_empty());
    assert!(fake.notifications().is_empty());
}

fn default_save_dir() -> PathBuf {
    Path::new(DEFAULT_PICTURES_DIR).join("BetterPrtSc")
}

/// 在 `region` 里画了一个框的标注捕获，时间是 2026-10-24 09:05:07。
fn annotated_with_rectangle(actions: Vec<PostCaptureAction>) -> (FakePlatform, ResidentInstance) {
    let fake = single_display().with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7));
    let mut instance = start(
        &fake,
        Command::Capture(CaptureRequest::new(CaptureMode::Region, actions)),
    );
    feed(
        &mut instance,
        drag_region(&fake, Rect::new(100, 80, 300, 200)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );
    (fake, instance)
}

#[test]
fn auto_save_button_saves_annotated_screenshot_by_filename_template_and_notifies() {
    let (fake, mut instance) = annotated_with_rectangle(vec![PostCaptureAction::Annotate]);

    feed(&mut instance, fake.click_text("自动存储"));

    let path = default_save_dir().join("2026-10-24 09-05-07.png");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    let saved = decode::png(&fake.file(&path).unwrap());
    assert!(is_red(saved.pixel(50, 40)), "存下的截图包含标注");
    assert!(fake.clipboard().is_empty(), "只选了自动存储，不复制");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "截图已保存");
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenFile(path))
    );
    assert!(!fake.overlay_visible());
}

#[test]
fn bound_auto_save_also_runs_when_user_copies_and_clipboard_carries_the_file() {
    let (fake, mut instance) = annotated_with_rectangle(vec![
        PostCaptureAction::Annotate,
        PostCaptureAction::AutoSave,
    ]);

    feed(&mut instance, fake.click_text("复制"));

    let path = default_save_dir().join("2026-10-24 09-05-07.png");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB", "Files"]);
    assert_eq!(clipboard.files(), Some(vec![path]));
    assert!(is_red(clipboard.png_image().unwrap().pixel(50, 40)));
}

#[test]
fn save_as_writes_annotated_screenshot_to_the_chosen_path_in_its_format() {
    let (fake, mut instance) = annotated_with_rectangle(vec![PostCaptureAction::Annotate]);
    let fake = fake.with_dir(r"D:\Shots");
    let chosen = PathBuf::from(r"D:\Shots\bug.jpg");
    fake.answer_save_as(Some(chosen.clone()));

    feed(&mut instance, fake.click_text("另存为"));

    assert_eq!(
        fake.save_as_dialogs(),
        [SaveAsOptions {
            file_name: "2026-10-24 09-05-07.png".to_owned(),
            folder: Some(default_save_dir()),
            file_types: vec![
                file_type("PNG 图像", &["png"]),
                file_type("JPEG 图像", &["jpg", "jpeg"]),
                file_type("WebP 图像", &["webp"]),
                file_type("AVIF 图像", &["avif"]),
            ],
            default_file_type: 0,
        }]
    );
    assert_eq!(fake.files(), std::slice::from_ref(&chosen));
    let saved = decode::jpeg(&fake.file(&chosen).unwrap());
    assert_eq!((saved.width(), saved.height()), (300, 200));
    let [r, g, b, _] = saved.pixel(100, 40);
    assert!(
        r > 180 && g < 100 && b < 100,
        "JPEG 里的框线应仍是红色：{r},{g},{b}"
    );
    assert!(fake.clipboard().is_empty(), "另存为不写剪贴板");
    assert!(
        fake.notifications().is_empty(),
        "用户自己选的位置，不另发通知"
    );
    assert!(!fake.overlay_visible());
}

fn file_type(name: &str, extensions: &[&str]) -> FileType {
    FileType {
        name: name.to_owned(),
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
    }
}

#[test]
fn save_as_overwrites_the_file_user_confirmed_and_defaults_to_output_format() {
    let existing = PathBuf::from(r"D:\Shots\bug.webp");
    let fake = single_display().with_file(&existing, b"old".to_vec());
    let mut request = CaptureRequest::new(CaptureMode::Region, vec![PostCaptureAction::Annotate]);
    request.output.output_format = OutputFormat::WebP;
    let mut instance = start(&fake, Command::Capture(request));
    feed(
        &mut instance,
        drag_region(&fake, Rect::new(100, 80, 300, 200)),
    );
    fake.answer_save_as(Some(existing.clone()));

    feed(&mut instance, fake.click_text("另存为"));

    let dialog = &fake.save_as_dialogs()[0];
    assert!(dialog.file_name.ends_with(".webp"), "{dialog:?}");
    assert_eq!(dialog.default_file_type, 2);
    let saved = decode::webp(&fake.file(&existing).unwrap());
    assert!(saved == desktop_pattern(Rect::new(100, 80, 300, 200)));
}

#[test]
fn cancelling_save_as_dialog_returns_to_annotating() {
    let (fake, mut instance) = annotated_with_rectangle(vec![PostCaptureAction::Annotate]);
    fake.answer_save_as(None);

    feed(&mut instance, fake.click_text("另存为"));

    assert!(fake.overlay_visible(), "取消对话框后回到标注");
    assert!(fake.files().is_empty());
    let (_, canvas) = fake.overlay_canvas().unwrap();
    assert!(is_red(canvas.pixel(50, 40)), "已画的标注还在");

    feed(&mut instance, fake.click_text("复制"));
    assert!(is_red(fake.clipboard().png_image().unwrap().pixel(50, 40)));
}

#[test]
fn failed_save_as_is_notified() {
    let (fake, mut instance) = annotated_with_rectangle(vec![PostCaptureAction::Annotate]);
    let read_only = PathBuf::from(r"D:\ReadOnly");
    let fake = fake.with_read_only_dir(&read_only);
    fake.answer_save_as(Some(read_only.join("a.png")));

    let result = fake
        .click_text("另存为")
        .into_iter()
        .map(|event| instance.handle(event))
        .last()
        .unwrap();

    assert!(matches!(result, Err(CoreError::Platform(_))), "{result:?}");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "另存为失败");
    assert!(!fake.overlay_visible());
}

#[test]
fn display_capture_with_annotate_goes_straight_to_annotating_the_display_under_cursor() {
    let second = Rect::new(800, -100, 640, 480);
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_display(FakeDisplay::new(second))
        .with_cursor_at(Point::new(900, 0));
    let mut instance = start(&fake, annotate(CaptureMode::Display));
    for button in toolbar_rects(&fake) {
        assert!(
            contains(second, button),
            "工具条在鼠标所在的显示器上：{button:?}"
        );
    }

    feed(&mut instance, fake.click_text("箭头"));
    feed(
        &mut instance,
        fake.drag(Point::new(850, 0), Point::new(1050, 0)),
    );
    feed(&mut instance, fake.click_text("复制"));

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (640, 480));
    assert!(
        is_red(png.pixel(150, 100)),
        "截图坐标 (50,100)→(250,100) 的箭头"
    );
    assert_eq!(png.pixel(10, 10), desktop_pattern(second).pixel(10, 10));
}

#[test]
fn virtual_desktop_capture_with_annotate_annotates_the_whole_virtual_desktop() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_display(FakeDisplay::new(Rect::new(800, 0, 640, 480)));
    let mut instance = start(&fake, annotate(CaptureMode::VirtualDesktop));

    feed(&mut instance, fake.click_text("复制"));

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (1440, 600));
}
