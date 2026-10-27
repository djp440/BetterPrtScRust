//! 标注：截后动作为等待确认时，选区旁浮现标注工具条；用户画框、箭头，最后确认（复制到剪贴板）
//! 或另存为结束这次捕获。
//!
//! 用户在遮罩上的操作由假平台模拟：工具条按钮用 `click_text`（点击遮罩上写着这段文字的地方），
//! 画标注用 `drag`。期望图像来自假平台的 `desktop_pattern`。

use std::path::{Path, PathBuf};

use bps_core::{
    CaptureMode, CaptureRequest, Command, CoreError, OutputFormat, PostCaptureAction,
    ResidentInstance,
};
use bps_platform::{
    Color, FileType, LocalDateTime, NotificationClick, OverlayItem, PlatformEvent, Point,
    PointerButton, Rect, SaveAsOptions,
};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, decode, desktop_pattern};

/// 工具条上的全部按钮文字。
const TOOLBAR: [&str; 11] = [
    "矩形",
    "箭头",
    "马赛克",
    "文字",
    "录屏",
    "长截屏",
    "OCR",
    "翻译",
    "取消",
    "另存为",
    "确认",
];

/// 工具条上的 5 种预设颜色（第一种是默认的红色）。
const PALETTE: [Color; 5] = [
    Color::rgba(240, 40, 40, 255),
    Color::rgba(255, 190, 0, 255),
    Color::rgba(40, 170, 70, 255),
    Color::rgba(30, 110, 240, 255),
    Color::rgba(0, 0, 0, 255),
];
const BLUE: Color = PALETTE[3];

fn annotate(mode: CaptureMode) -> Command {
    Command::Capture(CaptureRequest::new(
        mode,
        PostCaptureAction::AwaitConfirmation,
    ))
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

/// 工具条上显示 `color` 的色块（正方形的填充）。
fn swatch(fake: &FakePlatform, color: Color) -> Rect {
    let scene = fake.overlay_scene().expect("遮罩应显示着");
    scene
        .items
        .iter()
        .find_map(|item| match *item {
            OverlayItem::Fill { rect, color: c } if c == color && rect.width == rect.height => {
                Some(rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("工具条上没有 {color:?} 色块"))
}

/// 工具条上三档线宽的线条样本（白色填充），从细到粗。
fn thickness_samples(fake: &FakePlatform) -> Vec<Rect> {
    let scene = fake.overlay_scene().expect("遮罩应显示着");
    let mut samples: Vec<Rect> = scene
        .items
        .iter()
        .filter_map(|item| match *item {
            // 选区的调整手柄也是白色填充，但是正方形；线条样本是扁的。
            OverlayItem::Fill { rect, color }
                if color == Color::rgba(255, 255, 255, 255) && rect.width > rect.height =>
            {
                Some(rect)
            }
            _ => None,
        })
        .collect();
    samples.sort_by_key(|r| r.height);
    assert_eq!(samples.len(), 3, "工具条上应有三档线宽：{samples:?}");
    samples
}

fn click_on(fake: &FakePlatform, rect: Rect) -> Vec<PlatformEvent> {
    let center = Point::new(
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    fake.click(PointerButton::Left, center)
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.intersection(inner) == Some(inner)
}

#[test]
fn toolbar_appears_right_below_selected_region_when_capture_annotates() {
    let fake = single_display();
    let mut instance = start(&fake, annotate(CaptureMode::Region));
    // 选区比工具条宽，工具条才能与选区右边对齐（窄选区时工具条被挪进显示器）。
    let region = Rect::new(200, 80, 560, 200);

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
        fake.overlay_scene().unwrap().text_rect("确认").is_none(),
        "拖拽框选期间不显示工具条"
    );

    let fake = single_display();
    let mut instance = start(
        &fake,
        Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::CopyToClipboard,
        )),
    );
    feed(
        &mut instance,
        drag_region(&fake, Rect::new(100, 80, 300, 200)),
    );
    assert!(
        !fake.overlay_visible(),
        "不含标注的复制捕获松开即完成，没有工具条"
    );
}

#[test]
fn copy_button_without_annotations_copies_the_region() {
    let fake = single_display();
    let mut instance = start(&fake, annotate(CaptureMode::Region));
    let region = Rect::new(100, 80, 300, 200);
    feed(&mut instance, drag_region(&fake, region));

    feed(&mut instance, fake.click_text("确认"));

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
    feed(&mut instance, fake.click_text("确认"));

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
    feed(&mut instance, fake.click_text("确认"));

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
    feed(&mut instance, fake.click_text("确认"));

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
fn mosaic_dragged_on_region_replaces_its_content_with_coarse_blocks() {
    let fake = single_display();
    let region = Rect::new(100, 80, 300, 200);
    let mut instance = annotating(&fake, region);

    feed(&mut instance, fake.click_text("马赛克"));
    // 截图坐标 (40,30)–(199,129)：160×100。
    feed(
        &mut instance,
        fake.drag(Point::new(140, 110), Point::new(299, 209)),
    );
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    let mosaic = Rect::new(40, 30, 160, 100);
    let mut colors = std::collections::HashSet::new();
    let mut unchanged = 0;
    for y in mosaic.y..mosaic.y + mosaic.height as i32 {
        for x in mosaic.x..mosaic.x + mosaic.width as i32 {
            let (x, y) = (x as u32, y as u32);
            unchanged += usize::from(png.pixel(x, y) == base.pixel(x, y));
            colors.insert(png.pixel(x, y));
        }
    }
    // 桌面图案每个像素颜色都不同、按坐标渐变，块的平均色最多与块中心那一个像素相同。
    assert!(unchanged <= 21 * 14, "{unchanged} 个像素保留原样");
    // 色块边长至少 8 像素时，160×100 里最多 (160/8+1)×(100/8+1) 种颜色。
    assert!(
        colors.len() <= 21 * 14,
        "颜色太多，不像马赛克：{}",
        colors.len()
    );
    for (x, y) in [(39, 30), (40, 29), (200, 129), (199, 130), (10, 10)] {
        assert_eq!(
            png.pixel(x, y),
            base.pixel(x, y),
            "马赛克以外 ({x},{y}) 不变"
        );
    }
}

#[test]
fn changing_color_and_thickness_affects_only_annotations_drawn_afterwards() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    // 默认红色、中档（4 像素）：截图坐标 (50,40)–(149,99)。
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );

    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));
    feed(&mut instance, click_on(&fake, thickness_samples(&fake)[2]));
    // 蓝色、粗档（6 像素）：截图坐标 (60,150)–(259,229)。
    feed(
        &mut instance,
        fake.drag(Point::new(160, 230), Point::new(359, 309)),
    );
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    let rgba = |c: Color| [c.r, c.g, c.b, c.a];
    assert_eq!(
        png.pixel(53, 70),
        rgba(PALETTE[0]),
        "先画的框仍是红色、4 像素宽"
    );
    assert_eq!(png.pixel(54, 70), base.pixel(54, 70));
    assert_eq!(png.pixel(65, 190), rgba(BLUE), "后画的框是蓝色、6 像素宽");
    assert_eq!(png.pixel(66, 190), base.pixel(66, 190));
}

#[test]
fn thin_thickness_draws_two_pixel_lines() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, click_on(&fake, thickness_samples(&fake)[0]));
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!(png.pixel(51, 70), [240, 40, 40, 255]);
    assert_eq!(png.pixel(52, 70), desktop_pattern(region).pixel(52, 70));
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
fn annotated_with_rectangle(auto_save: bool) -> (FakePlatform, ResidentInstance) {
    let fake = single_display().with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7));
    let mut request =
        CaptureRequest::new(CaptureMode::Region, PostCaptureAction::AwaitConfirmation);
    request.auto_save = auto_save;
    let mut instance = start(&fake, Command::Capture(request));
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
fn auto_save_runs_on_confirm_by_filename_template_and_clipboard_carries_the_file() {
    let (fake, mut instance) = annotated_with_rectangle(true);

    feed(&mut instance, fake.click_text("确认"));

    let path = default_save_dir().join("2026-10-24 09-05-07.png");
    assert_eq!(fake.files(), std::slice::from_ref(&path));
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["PNG", "DIB", "Files"]);
    assert_eq!(clipboard.files(), Some(vec![path.clone()]));
    assert!(is_red(clipboard.png_image().unwrap().pixel(50, 40)));
    let saved = decode::png(&fake.file(&path).unwrap());
    assert!(is_red(saved.pixel(50, 40)), "存下的截图包含标注");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "截图已保存");
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenFile(path))
    );
}

#[test]
fn save_as_writes_annotated_screenshot_to_the_chosen_path_in_its_format() {
    let (fake, mut instance) = annotated_with_rectangle(false);
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
    let mut request =
        CaptureRequest::new(CaptureMode::Region, PostCaptureAction::AwaitConfirmation);
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
    let (fake, mut instance) = annotated_with_rectangle(false);
    fake.answer_save_as(None);

    feed(&mut instance, fake.click_text("另存为"));

    assert!(fake.overlay_visible(), "取消对话框后回到标注");
    assert!(fake.files().is_empty());
    let (_, canvas) = fake.overlay_canvas().unwrap();
    assert!(is_red(canvas.pixel(50, 40)), "已画的标注还在");

    feed(&mut instance, fake.click_text("确认"));
    assert!(is_red(fake.clipboard().png_image().unwrap().pixel(50, 40)));
}

#[test]
fn failed_save_as_is_notified() {
    let (fake, mut instance) = annotated_with_rectangle(false);
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
    feed(&mut instance, fake.click_text("确认"));

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

    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    assert_eq!((png.width(), png.height()), (1440, 600));
}

/// 默认（中档）字号 22 时假平台的字形（`bps_platform_fake::text_mask`）：行高 28，汉字宽 22、
/// ASCII 宽 13；字形方块在字符格内左右缩进 1、上下缩进 4。以下坐标都是截图坐标，文字原点
/// 在 (50,40)，即遮罩上的 (150,120)。
const TEXT_AT: Point = Point::new(150, 120);

fn caret(fake: &FakePlatform) -> Option<Rect> {
    fake.overlay_scene().expect("遮罩应显示着").text_caret
}

/// 选好文字工具、在 (150,120) 单击开始输入的标注捕获。
fn typing(fake: &FakePlatform, region: Rect) -> ResidentInstance {
    let mut instance = annotating(fake, region);
    feed(&mut instance, fake.click_text("文字"));
    feed(&mut instance, fake.click(PointerButton::Left, TEXT_AT));
    instance
}

#[test]
fn chinese_text_typed_with_input_method_is_drawn_where_the_user_clicked() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("文字"));
    assert_eq!(caret(&fake), None, "还没单击时不接受文字输入");

    feed(&mut instance, fake.click(PointerButton::Left, TEXT_AT));
    assert_eq!(
        caret(&fake),
        Some(Rect::new(150, 120, 1, 28)),
        "单击处开始输入：输入法打开，候选窗口跟着插入点"
    );
    feed(&mut instance, fake.type_text("你好"));
    assert_eq!(
        caret(&fake),
        Some(Rect::new(194, 120, 1, 28)),
        "插入点在“好”之后"
    );
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    assert!(is_red(png.pixel(60, 54)), "“你”");
    assert!(is_red(png.pixel(82, 54)), "“好”");
    for (x, y) in [(71, 54), (100, 54), (60, 42), (60, 65), (45, 54)] {
        assert_eq!(png.pixel(x, y), base.pixel(x, y), "字形以外 ({x},{y})");
    }
}

#[test]
fn backspace_deletes_and_enter_starts_a_new_line_while_typing() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);

    feed(&mut instance, fake.type_text("你好"));
    feed(
        &mut instance,
        vec![
            fake.press_key(bps_platform::OverlayKey::Backspace),
            fake.press_key(bps_platform::OverlayKey::Enter),
        ],
    );
    assert!(
        fake.overlay_visible(),
        "输入文字时 Enter 是换行，不结束捕获"
    );
    assert!(fake.clipboard().is_empty());
    feed(&mut instance, fake.type_text("A"));
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    assert!(is_red(png.pixel(60, 54)), "“你”还在");
    assert_eq!(png.pixel(82, 54), base.pixel(82, 54), "“好”已删除");
    assert!(is_red(png.pixel(55, 80)), "第二行的“A”");
}

#[test]
fn escape_finishes_typing_without_cancelling_the_capture() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);
    feed(&mut instance, fake.type_text("你"));

    feed(
        &mut instance,
        vec![fake.press_key(bps_platform::OverlayKey::Escape)],
    );
    assert!(fake.overlay_visible(), "Esc 只结束输入");
    assert_eq!(caret(&fake), None, "输入法关闭");
    feed(&mut instance, fake.type_text("好"));
    feed(
        &mut instance,
        vec![fake.press_key(bps_platform::OverlayKey::Enter)],
    );

    let png = fake.clipboard().png_image().expect("不在输入时 Enter 复制");
    assert!(is_red(png.pixel(60, 54)));
    assert_eq!(
        png.pixel(82, 54),
        desktop_pattern(region).pixel(82, 54),
        "结束输入后打的字不算"
    );
}

#[test]
fn clicking_elsewhere_starts_another_text_and_style_changes_apply_to_the_text_being_typed() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);
    feed(&mut instance, fake.type_text("你"));

    // 截图坐标 (50,170) 开始第二段文字，再改成蓝色、粗档（字号 30：行高 38，上下缩进 6）。
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(150, 250)),
    );
    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));
    feed(&mut instance, click_on(&fake, thickness_samples(&fake)[2]));
    assert_eq!(
        caret(&fake),
        Some(Rect::new(150, 250, 1, 38)),
        "改样式不打断输入"
    );
    feed(&mut instance, fake.type_text("好"));
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    let base = desktop_pattern(region);
    assert!(is_red(png.pixel(60, 54)), "第一段仍是红色");
    assert_eq!(png.pixel(75, 54), base.pixel(75, 54), "第一段仍是中档字号");
    assert_eq!(png.pixel(60, 180), [30, 110, 240, 255], "第二段是蓝色");
    assert_eq!(png.pixel(75, 200), [30, 110, 240, 255], "第二段是粗档字号");
}

#[test]
fn switching_tool_finishes_typing_and_typing_is_ignored_by_other_tools() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);
    feed(&mut instance, fake.type_text("你"));

    feed(&mut instance, fake.click_text("矩形"));
    assert_eq!(caret(&fake), None);
    feed(&mut instance, fake.type_text("好"));
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    assert!(is_red(png.pixel(60, 54)), "切换工具时已输入的文字保留");
    assert_eq!(png.pixel(82, 54), desktop_pattern(region).pixel(82, 54));
}

#[test]
fn overlay_shows_exactly_the_screenshot_with_mosaic_and_text_that_will_be_produced() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(130, 100), Point::new(260, 200)),
    );
    // 马赛克盖住框的一部分，色块从拖拽起点排起，与截图边缘不对齐。
    feed(&mut instance, fake.click_text("马赛克"));
    feed(
        &mut instance,
        fake.drag(Point::new(203, 147), Point::new(371, 288)),
    );
    // 第一段文字写在马赛克上，第二段在输入中途被马赛克的拖拽改动过的地方以外。
    feed(&mut instance, fake.click_text("文字"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(220, 160)),
    );
    feed(&mut instance, fake.type_text("马赛克上的字"));
    feed(&mut instance, fake.click(PointerButton::Left, TEXT_AT));
    feed(&mut instance, fake.type_text("Hi 你好"));

    let (area, shown) = fake.overlay_canvas().expect("标注时遮罩上应有画布");
    assert_eq!(area, region);
    feed(&mut instance, fake.click_text("确认"));

    let png = fake.clipboard().png_image().unwrap();
    assert!(
        png == shown,
        "屏幕上看到的应与最终截图逐像素一致（插入点不在截图里）"
    );
}

// ---- 撤销 / 重做与再编辑 ----

use bps_platform::{Modifiers, OverlayInput, OverlayKey};

const ORIGINAL_BOX_EDGE: Point = Point::new(151, 150);

/// 区域 (100,80,400,300)，已选好“矩形”并画了一个红色粗细中档的框：屏幕 (150,120)–(249,179)，
/// 即截图坐标 (50,40)–(149,99)，左边线在截图 x=50..53。
fn with_box() -> (FakePlatform, ResidentInstance, Rect) {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = annotating(&fake, region);
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(150, 120), Point::new(249, 179)),
    );
    (fake, instance, region)
}

fn press(instance: &mut ResidentInstance, fake: &FakePlatform, key: OverlayKey, m: Modifiers) {
    instance
        .handle(fake.press_key_with(key, m))
        .expect("按键应被处理");
}

fn copy_result(fake: &FakePlatform, instance: &mut ResidentInstance) -> bps_platform::Image {
    feed(instance, fake.click_text("确认"));
    fake.clipboard().png_image().expect("应复制到剪贴板")
}

fn canvas(fake: &FakePlatform) -> bps_platform::Image {
    fake.overlay_canvas().expect("应显示画布").1
}

#[test]
fn undo_and_redo_buttons_step_through_added_annotations() {
    let (fake, mut instance, region) = with_box();
    let base = desktop_pattern(region);
    feed(&mut instance, fake.click_text("箭头"));
    feed(
        &mut instance,
        fake.drag(Point::new(120, 300), Point::new(400, 300)),
    );
    assert!(is_red(canvas(&fake).pixel(100, 220)), "箭头已画上");

    feed(&mut instance, fake.click_text("撤销"));
    let shown = canvas(&fake);
    assert!(shown.pixel(100, 220) == base.pixel(100, 220), "箭头被撤销");
    assert!(is_red(shown.pixel(51, 70)), "框还在");

    feed(&mut instance, fake.click_text("撤销"));
    assert!(
        canvas(&fake).pixel(51, 70) == base.pixel(51, 70),
        "框也被撤销"
    );

    feed(&mut instance, fake.click_text("重做"));
    feed(&mut instance, fake.click_text("重做"));
    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 70)), "重做后框回来了");
    assert!(is_red(png.pixel(100, 220)), "重做后箭头回来了");
}

#[test]
fn ctrl_z_undoes_and_ctrl_y_or_ctrl_shift_z_redoes() {
    let (fake, mut instance, region) = with_box();
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    assert!(
        canvas(&fake) == desktop_pattern(region),
        "Ctrl+Z 撤销了画框"
    );
    press(&mut instance, &fake, OverlayKey::Y, Modifiers::CTRL);
    assert!(is_red(canvas(&fake).pixel(51, 70)), "Ctrl+Y 重做");
    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    press(&mut instance, &fake, OverlayKey::Z, ctrl_shift);
    assert!(is_red(canvas(&fake).pixel(51, 70)), "Ctrl+Shift+Z 重做");

    // 没按 Ctrl 的 Z 什么也不做。
    press(&mut instance, &fake, OverlayKey::Z, Modifiers::NONE);
    assert!(is_red(canvas(&fake).pixel(51, 70)));
    assert!(fake.overlay_visible(), "撤销、重做不结束捕获");
}

#[test]
fn undoing_or_redoing_with_nothing_to_do_changes_nothing() {
    let (fake, mut instance, region) = with_box();
    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    feed(&mut instance, fake.click_text("重做"));
    feed(&mut instance, fake.click_text("重做"));
    feed(&mut instance, fake.click_text("重做"));

    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 70)));
    assert!(png.pixel(10, 10) == desktop_pattern(region).pixel(10, 10));
}

#[test]
fn selected_annotation_can_be_dragged_and_the_whole_drag_is_one_undo_step() {
    let (fake, mut instance, region) = with_box();
    let base = desktop_pattern(region);
    feed(&mut instance, fake.click_text("选择"));

    // 抓住框的左边线，向右下拖 (30,20)：截图坐标 (80,60)–(179,119)。
    feed(
        &mut instance,
        fake.drag(ORIGINAL_BOX_EDGE, Point::new(181, 170)),
    );
    let shown = canvas(&fake);
    assert!(is_red(shown.pixel(81, 90)), "屏幕上框已移到新位置");
    assert!(shown.pixel(51, 70) == base.pixel(51, 70), "原位置空了");

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    let shown = canvas(&fake);
    assert!(is_red(shown.pixel(51, 70)), "一次撤销就回到拖动之前");
    assert!(shown.pixel(81, 90) == base.pixel(81, 90));
    press(&mut instance, &fake, OverlayKey::Y, Modifiers::CTRL);

    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(81, 90)));
    assert!(is_red(png.pixel(178, 118)), "移动后的右下角");
    assert!(png.pixel(51, 70) == base.pixel(51, 70));
}

#[test]
fn dragging_follows_the_pointer_while_the_button_is_down() {
    let (fake, mut instance, region) = with_box();
    feed(&mut instance, fake.click_text("选择"));
    let events = vec![
        fake.overlay_input(OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: ORIGINAL_BOX_EDGE,
            modifiers: Modifiers::NONE,
            double_click: false,
        }),
        fake.overlay_input(OverlayInput::PointerMoved {
            position: Point::new(171, 160),
            modifiers: Modifiers::NONE,
        }),
    ];
    feed(&mut instance, events);

    let shown = canvas(&fake);
    assert!(is_red(shown.pixel(71, 80)), "按住拖动时就跟着指针");
    assert!(shown.pixel(51, 70) == desktop_pattern(region).pixel(51, 70));
}

#[test]
fn delete_removes_the_selected_annotation_and_undo_restores_it() {
    let (fake, mut instance, region) = with_box();
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, ORIGINAL_BOX_EDGE),
    );

    press(&mut instance, &fake, OverlayKey::Delete, Modifiers::NONE);
    assert!(canvas(&fake) == desktop_pattern(region), "框被删除");
    assert!(fake.overlay_visible());

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    assert!(is_red(canvas(&fake).pixel(51, 70)), "撤销删除");
}

#[test]
fn delete_without_a_selection_or_after_clicking_empty_space_does_nothing() {
    let (fake, mut instance, _) = with_box();
    press(&mut instance, &fake, OverlayKey::Delete, Modifiers::NONE);
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, ORIGINAL_BOX_EDGE),
    );
    // 点空白处取消选中（框内部是空的，点不中）。
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(200, 150)),
    );
    press(&mut instance, &fake, OverlayKey::Delete, Modifiers::NONE);

    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 70)), "框还在");
}

#[test]
fn color_and_thickness_buttons_restyle_the_selected_annotation_undoably() {
    let (fake, mut instance, region) = with_box();
    let base = desktop_pattern(region);
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, ORIGINAL_BOX_EDGE),
    );

    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));
    let shown = canvas(&fake);
    assert_eq!(shown.pixel(51, 70), [30, 110, 240, 255], "框改成蓝色");
    assert!(shown.pixel(54, 70) == base.pixel(54, 70), "线宽没变");

    feed(&mut instance, click_on(&fake, thickness_samples(&fake)[2]));
    let shown = canvas(&fake);
    assert_eq!(shown.pixel(55, 70), [30, 110, 240, 255], "粗档线宽 6 像素");
    assert!(shown.pixel(56, 70) == base.pixel(56, 70));

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    let shown = canvas(&fake);
    assert!(shown.pixel(55, 70) == base.pixel(55, 70), "撤销线宽");
    assert_eq!(shown.pixel(51, 70), [30, 110, 240, 255]);
    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    assert!(is_red(canvas(&fake).pixel(51, 70)), "撤销颜色");
}

#[test]
fn restyling_without_a_selection_only_affects_later_annotations() {
    let (fake, mut instance, _) = with_box();
    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));

    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 70)), "没有选中时已画的框不变");
}

#[test]
fn clicking_a_toolbar_button_keeps_the_selection() {
    let (fake, mut instance, _) = with_box();
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, ORIGINAL_BOX_EDGE),
    );
    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));
    feed(&mut instance, click_on(&fake, swatch(&fake, PALETTE[1])));

    let png = copy_result(&fake, &mut instance);
    assert_eq!(
        png.pixel(51, 70),
        [255, 190, 0, 255],
        "两次改色都作用在同一个框上"
    );
}

#[test]
fn topmost_annotation_is_selected_and_moved_alone() {
    let (fake, mut instance, region) = with_box();
    let base = desktop_pattern(region);
    // 再画一个与第一个框左边线重叠的框：截图坐标 (50,60)–(120,120)。
    feed(
        &mut instance,
        fake.drag(Point::new(150, 140), Point::new(220, 200)),
    );
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.drag(Point::new(151, 170), Point::new(191, 170)),
    );

    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 50)), "下面那个框的左边线不动");
    assert!(
        png.pixel(51, 110) == base.pixel(51, 110),
        "上面那个框原来的位置空了"
    );
    assert!(is_red(png.pixel(91, 110)), "上面那个框右移了 40 像素");
}

#[test]
fn text_can_be_selected_moved_recolored_and_resized() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);
    feed(&mut instance, fake.type_text("你好"));
    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);
    let base = desktop_pattern(region);

    feed(&mut instance, fake.click_text("选择"));
    // 点在“你”的字形上选中，再改成粗档（字号 30）和蓝色。
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(160, 134)),
    );
    feed(&mut instance, click_on(&fake, thickness_samples(&fake)[2]));
    feed(&mut instance, click_on(&fake, swatch(&fake, BLUE)));
    assert_eq!(
        canvas(&fake).pixel(60, 68),
        [30, 110, 240, 255],
        "字号变大、颜色变蓝"
    );

    // 向右拖 40 像素。
    feed(
        &mut instance,
        fake.drag(Point::new(160, 134), Point::new(200, 134)),
    );
    let shown = canvas(&fake);
    assert_eq!(shown.pixel(100, 54), [30, 110, 240, 255]);
    assert!(shown.pixel(60, 54) == base.pixel(60, 54), "原位置空了");

    // 撤销移动、颜色、字号：回到最初的字。
    for _ in 0..3 {
        press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    }
    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(60, 54)), "“你”回到原位、原色");
    assert!(png.pixel(60, 68) == base.pixel(60, 68), "字号也回到中档");
}

#[test]
fn finished_text_is_one_undo_step() {
    let fake = single_display();
    let region = Rect::new(100, 80, 400, 300);
    let mut instance = typing(&fake, region);
    feed(&mut instance, fake.type_text("你好"));
    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    assert!(
        canvas(&fake) == desktop_pattern(region),
        "整段文字一起被撤销"
    );
}

#[test]
fn undo_and_delete_keys_while_typing_do_not_touch_the_annotations() {
    let (fake, mut instance, _) = with_box();
    feed(&mut instance, fake.click_text("文字"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(300, 250)),
    );
    feed(&mut instance, fake.type_text("a"));

    press(&mut instance, &fake, OverlayKey::Z, Modifiers::CTRL);
    press(&mut instance, &fake, OverlayKey::Delete, Modifiers::NONE);
    let png = copy_result(&fake, &mut instance);
    assert!(is_red(png.pixel(51, 70)), "打字时快捷键不影响已有的框");
}
