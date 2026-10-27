//! 长截屏：截后动作包含长截屏时，用户先框选区域，按“开始”后自己竖直滚动区域里的内容，
//! 程序持续采样并拼接；区域旁边实时显示拼接预览；按“完成”结束（高度到达上限时自动停止并
//! 提示），之后进入这条热键绑定后续的截后动作（标注、复制、自动存储）。
//!
//! 假平台模拟桌面上一个可以滚动的窗口：它的内容区（视口）显示一张合成的长文档，
//! `show` 把文档滚到某个位置（改变假显示器的画面），`fake.sample_region()` 模拟平台在
//! 画面变化后采到一帧。期望的长截屏直接从文档裁剪得到。

use bps_core::{
    CaptureMode, CaptureRequest, CliStatus, Command, CoreError, DirectFunction, Flow,
    HotkeyBinding, Locale, LongScreenshot, PostCaptureAction, ResidentInstance, run_cli,
};
use bps_platform::{
    Hotkey, Image, Key, Modifiers, OverlayItem, OverlayKey, PlatformEvent, Point, PointerButton,
    Rect,
};
use bps_platform_fake::{
    FakeDisplay, FakePlatform, FakeResidentChannel, FakeWindow, decode, desktop_pattern,
};

/// 显示器。
const SCREEN: Rect = Rect::new(0, 0, 640, 400);
/// 可滚动窗口的内容区（视口），也是用户框选的区域。
const VIEWPORT: Rect = Rect::new(100, 60, 120, 150);

/// 合成的长文档：宽度等于视口，每行内容各不相同，每 50 行里有 8 行空白。
fn document(height: u32) -> Image {
    document_of_width(VIEWPORT.width, height)
}

fn document_of_width(width: u32, height: u32) -> Image {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let blank = y % 50 >= 42;
        for x in 0..width {
            let pixel = if blank {
                [250, 250, 250, 255]
            } else {
                let h = mix(u64::from(x / 3) << 32 | u64::from(y));
                [h as u8, (h >> 8) as u8, (h >> 16) as u8, 255]
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    Image::from_rgba(width, height, pixels)
}

fn mix(mut v: u64) -> u64 {
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51_afd7_ed55_8ccd);
    v ^= v >> 33;
    v = v.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    v ^ (v >> 33)
}

/// `image` 的第 `from` 行（含）到第 `to` 行（不含）。
fn rows(image: &Image, from: u32, to: u32) -> Image {
    let row = (image.width() * 4) as usize;
    let pixels = image.rgba()[from as usize * row..to as usize * row].to_vec();
    Image::from_rgba(image.width(), to - from, pixels)
}

/// 桌面上的一个可滚动窗口：视口 `viewport` 里显示长文档 `doc`。
struct ScrollingWindow {
    fake: FakePlatform,
    doc: Image,
    viewport: Rect,
    screen: Rect,
}

impl ScrollingWindow {
    /// 把文档滚到 `offset`：视口里显示文档从第 `offset` 行起的内容，桌面其余部分不变。
    fn show(&self, offset: u32) {
        let mut screen = desktop_pattern(self.screen).into_rgba();
        let width = self.screen.width as usize;
        let content = rows(&self.doc, offset, offset + self.viewport.height);
        let row = self.viewport.width as usize * 4;
        for (y, line) in content.rgba().chunks_exact(row).enumerate() {
            let sy = (self.viewport.y - self.screen.y) as usize + y;
            let sx = (self.viewport.x - self.screen.x) as usize;
            let start = (sy * width + sx) * 4;
            screen[start..start + row].copy_from_slice(line);
        }
        self.fake.change_display_image(
            0,
            Image::from_rgba(self.screen.width, self.screen.height, screen),
        );
    }

    /// 用户把内容滚到 `offset`，平台随即采到一帧。
    fn scroll_to(&self, instance: &mut ResidentInstance, offset: u32) {
        self.show(offset);
        let event = self.fake.sample_region().expect("应正在采样");
        instance.handle(event).unwrap();
    }
}

fn desktop_with_window(doc: Image, viewport: Rect) -> ScrollingWindow {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(SCREEN).primary())
        .with_cursor_at(Point::new(30, 20));
    let window = ScrollingWindow {
        fake,
        doc,
        viewport,
        screen: SCREEN,
    };
    window.show(0);
    window
}

/// 直达功能的长截屏（热键直接进入）。
fn long_screenshot() -> Command {
    Command::Capture(CaptureRequest::function(
        CaptureMode::Region,
        DirectFunction::LongScreenshot,
    ))
}

/// 从等待确认界面转入长截屏：先框选 [`VIEWPORT`]，再点工具条上的“长截屏”（以选区直接开始采样）。
fn start_from_confirmation(fake: &FakePlatform) -> ResidentInstance {
    let mut instance = start(
        fake,
        Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )),
    );
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("长截屏"));
    instance
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
    fake.drag(
        Point::new(region.x, region.y),
        Point::new(
            region.x + region.width as i32 - 1,
            region.y + region.height as i32 - 1,
        ),
    )
}

fn has_text(fake: &FakePlatform, text: &str) -> bool {
    fake.overlay_scene()
        .is_some_and(|scene| scene.text_rect(text).is_some())
}

/// 遮罩上是否有文字为 `text` 的标签（拼接状态）。
fn has_label(fake: &FakePlatform, text: &str) -> bool {
    fake.overlay_scene().is_some_and(|scene| {
        scene
            .items
            .iter()
            .any(|item| matches!(item, OverlayItem::Label(label) if label.text == text))
    })
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.intersection(inner) == Some(inner)
}

fn assert_same(actual: &Image, expected: &Image) {
    assert_eq!(
        (actual.width(), actual.height()),
        (expected.width(), expected.height()),
        "长截屏尺寸不对"
    );
    let row = (expected.width() * 4) as usize;
    let first_bad = actual
        .rgba()
        .chunks_exact(row)
        .zip(expected.rgba().chunks_exact(row))
        .position(|(a, e)| a != e);
    assert_eq!(first_bad, None, "长截屏从这一行起与期望不同");
}

#[test]
fn start_and_cancel_buttons_appear_beside_selected_region() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());

    assert!(!has_text(fake, "开始"), "还没有选区时不显示“开始”");
    feed(&mut instance, drag_region(fake, VIEWPORT));

    let scene = fake.overlay_scene().unwrap();
    let start_button = scene.text_rect("开始").expect("应显示“开始”");
    let cancel_button = scene.text_rect("取消").expect("应显示“取消”");
    for button in [start_button, cancel_button] {
        assert!(button.intersection(VIEWPORT).is_none(), "按钮不应盖住区域");
        assert!(contains(SCREEN, button));
    }
    assert_eq!(fake.sampling_region(), None, "按“开始”之前不采样");
}

#[test]
fn clicking_a_window_selects_its_area_and_still_waits_for_start() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = window
        .fake
        .clone()
        .with_window(FakeWindow::new("文档", VIEWPORT));
    let mut instance = start(&fake, long_screenshot());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(150, 100)),
    );
    assert!(fake.overlay_visible(), "单击窗口后仍要确认才开始");
    assert!(has_text(&fake, "开始"));
    assert_eq!(fake.sampling_region(), None, "按“开始”之前不采样");

    feed(&mut instance, fake.click_text("开始"));
    assert_eq!(fake.sampling_region(), Some(VIEWPORT), "应采样窗口的范围");
}

#[test]
fn pressing_start_samples_the_region_and_lets_the_user_scroll_it() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));

    feed(&mut instance, fake.click_text("开始"));

    assert_eq!(fake.sampling_region(), Some(VIEWPORT));
    assert_eq!(
        fake.overlay_pass_through(),
        Some(VIEWPORT),
        "区域应透过遮罩，用户才能在里面滚动"
    );
    assert!(has_text(fake, "完成"));
    assert!(has_text(fake, "取消"));
    assert!(!has_text(fake, "开始"));
}

#[test]
fn finishing_copies_the_stitched_long_screenshot() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    for offset in (0..=600).step_by(20) {
        window.scroll_to(&mut instance, offset);
    }
    feed(&mut instance, fake.click_text("完成"));

    assert!(!fake.overlay_visible(), "完成后遮罩应关闭");
    assert_eq!(fake.sampling_region(), None, "完成后应停止采样");
    let copied = fake.clipboard().png_image().expect("剪贴板里应有 PNG");
    assert_same(&copied, &rows(&doc, 0, 600 + VIEWPORT.height));
}

#[test]
fn preview_beside_region_shows_stitched_size_while_scrolling() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));
    assert!(
        has_label(fake, "已拼接 120 × 150"),
        "一开始只有选区里的一帧"
    );

    for offset in (0..=200).step_by(25) {
        window.scroll_to(&mut instance, offset);
    }

    assert!(has_label(fake, "已拼接 120 × 350"));
    let (preview, image) = fake.overlay_canvas().expect("应显示拼接预览");
    assert!(preview.intersection(VIEWPORT).is_none(), "预览不应盖住区域");
    assert!(preview.x > VIEWPORT.x, "预览应在区域旁边（右侧放得下）");
    assert!(contains(SCREEN, preview));
    assert_eq!(
        (image.width(), image.height()),
        (preview.width, preview.height)
    );
    assert!(preview.width < VIEWPORT.width, "预览是缩小的");
}

#[test]
fn scrolling_too_fast_asks_user_to_scroll_back_and_recovers() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    window.scroll_to(&mut instance, 100);
    window.scroll_to(&mut instance, 500);
    assert!(has_label(fake, "滚动太快，请往回滚一点"));

    window.scroll_to(&mut instance, 200);
    assert!(!has_label(fake, "滚动太快，请往回滚一点"));
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    let copied = fake.clipboard().png_image().unwrap();
    assert_same(&copied, &rows(&doc, 0, 200 + VIEWPORT.height));
}

#[test]
fn reaching_the_height_limit_stops_stitching_and_tells_the_user() {
    let limit = LongScreenshot::MAX_HEIGHT;
    let doc = document(limit + 1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    let mut offset = 0;
    while fake.sampling_region().is_some() {
        offset += 100;
        assert!(
            offset + VIEWPORT.height <= doc.height(),
            "到达上限后应自动停止"
        );
        window.scroll_to(&mut instance, offset);
    }

    assert!(!fake.overlay_visible(), "自动停止后直接进入截后动作");
    let notification = fake.notifications().pop().expect("应提示已到达上限");
    assert_eq!(notification.title, "长截屏已到达高度上限");
    assert!(
        notification.body.contains(&limit.to_string()),
        "{notification:?}"
    );
    let copied = fake.clipboard().png_image().unwrap();
    assert_same(&copied, &rows(&doc, 0, limit));
}

#[test]
fn escape_while_scrolling_cancels_without_any_product() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));
    window.scroll_to(&mut instance, 100);

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert_eq!(fake.sampling_region(), None, "取消后应停止采样");
    assert!(fake.clipboard().is_empty());
    assert!(fake.notifications().is_empty());
    assert!(fake.sample_region().is_none(), "取消后不再有采样事件");
}

#[test]
fn cancel_button_while_scrolling_cancels_and_stops_sampling() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));
    window.scroll_to(&mut instance, 100);

    feed(&mut instance, fake.click_text("取消"));

    assert!(!fake.overlay_visible());
    assert_eq!(fake.sampling_region(), None);
    assert!(fake.clipboard().is_empty());
}

#[test]
fn enter_starts_scrolling_and_enter_again_finishes() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);
    assert_eq!(fake.sampling_region(), Some(VIEWPORT));
    window.scroll_to(&mut instance, 60);
    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    let copied = fake.clipboard().png_image().unwrap();
    assert_same(&copied, &rows(&doc, 0, 60 + VIEWPORT.height));
}

#[test]
fn binding_without_copy_or_auto_save_copies_the_long_screenshot() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    // 默认的 Ctrl+Shift+PrtSc 绑定只有长截屏一个截后动作。
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));
    window.scroll_to(&mut instance, 90);
    feed(&mut instance, fake.click_text("完成"));

    let copied = fake.clipboard().png_image().expect("应复制到剪贴板");
    assert_same(&copied, &rows(&doc, 0, 90 + VIEWPORT.height));
    assert!(fake.files().is_empty());
}

#[test]
fn auto_save_saves_the_long_screenshot_and_notifies() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(
        fake,
        Command::Capture(
            CaptureRequest::function(CaptureMode::Region, DirectFunction::LongScreenshot)
                .with_auto_save(),
        ),
    );
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));
    window.scroll_to(&mut instance, 120);
    feed(&mut instance, fake.click_text("完成"));

    let files = fake.files();
    assert_eq!(files.len(), 1, "{files:?}");
    let saved = decode::png(&fake.file(&files[0]).unwrap());
    assert_same(&saved, &rows(&doc, 0, 120 + VIEWPORT.height));
    assert_eq!(
        fake.clipboard().files(),
        Some(files),
        "长截屏总是复制，开了自动保存时剪贴板附带存下的文件"
    );
    assert_eq!(fake.notifications().len(), 1);
}

#[test]
fn annotating_a_long_screenshot_shows_it_scaled_down_and_produces_full_size() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start_from_confirmation(fake);
    for offset in (0..=600).step_by(30) {
        window.scroll_to(&mut instance, offset);
    }
    feed(&mut instance, fake.click_text("完成"));

    assert!(
        fake.overlay_visible(),
        "从等待确认转入的长截屏，完成后回到等待确认（标注）"
    );
    assert_eq!(fake.sampling_region(), None);
    assert_eq!(
        fake.overlay_pass_through(),
        None,
        "标注时整个遮罩都接收输入"
    );
    let (view, _) = fake.overlay_canvas().expect("应显示长截屏");
    assert!(contains(SCREEN, view), "{view:?}");
    assert!(view.height < 750, "比屏幕高的长截屏应缩小显示");

    // 在缩小的视图左上角画一个矩形，再复制。
    feed(&mut instance, fake.click_text("矩形"));
    let (from, to) = (
        Point::new(view.x + 4, view.y + 4),
        Point::new(view.x + 20, view.y + 30),
    );
    feed(&mut instance, fake.drag(from, to));
    feed(&mut instance, fake.click_text("确认"));

    assert!(!fake.overlay_visible());
    let copied = fake.clipboard().png_image().unwrap();
    assert_eq!(
        (copied.width(), copied.height()),
        (120, 750),
        "复制的是原尺寸"
    );
    let scale = 750.0 / f64::from(view.height);
    let (left, top) = ((4.5 * scale) as u32, (4.5 * scale) as u32);
    let bottom = (30.5 * scale) as u32;
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 80 && p[2] < 80;
    assert!(
        red(copied.pixel(left, (top + bottom) / 2)),
        "矩形左边应按比例画在原尺寸的长截屏上"
    );
    let below = bottom + 20;
    assert_same(&rows(&copied, below, 750), &rows(&doc, below, 750));
}

#[test]
fn sampling_failure_ends_the_capture_and_notifies() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    let failed = instance.handle(fake.fail_sampling("显示器已断开").unwrap());

    assert!(matches!(failed, Err(CoreError::Platform(_))), "{failed:?}");
    assert!(!fake.overlay_visible());
    assert_eq!(fake.sampling_region(), None);
    assert!(fake.clipboard().is_empty());
    let notification = fake.notifications().pop().expect("应通知捕获失败");
    assert!(
        notification.body.contains("显示器已断开"),
        "{notification:?}"
    );
}

#[test]
fn exiting_while_scrolling_stops_sampling_and_closes_overlay() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    assert_eq!(
        instance.handle(PlatformEvent::ExitRequested),
        Ok(Flow::Exit)
    );

    assert!(!fake.overlay_visible());
    assert_eq!(fake.sampling_region(), None);
    assert!(fake.clipboard().is_empty());
}

#[test]
fn another_capture_is_refused_while_scrolling() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(fake, long_screenshot());
    feed(&mut instance, drag_region(fake, VIEWPORT));
    feed(&mut instance, fake.click_text("开始"));

    let again = instance.execute(long_screenshot());

    assert_eq!(again, Err(CoreError::CaptureInProgress));
    assert_eq!(
        fake.sampling_region(),
        Some(VIEWPORT),
        "进行中的长截屏不受影响"
    );
}

#[test]
fn display_mode_starts_with_the_whole_display_selected() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start(
        fake,
        Command::Capture(CaptureRequest::function(
            CaptureMode::Display,
            DirectFunction::LongScreenshot,
        )),
    );

    feed(&mut instance, fake.click_text("开始"));

    assert_eq!(fake.sampling_region(), Some(SCREEN));
    let open = fake.overlay_pass_through().unwrap();
    let (preview, _) = fake.overlay_canvas().unwrap();
    let finish = fake.overlay_scene().unwrap().text_rect("完成").unwrap();
    assert!(contains(SCREEN, open));
    assert!(
        open.intersection(preview).is_none(),
        "预览盖在区域里时不透过遮罩"
    );
    assert!(
        open.intersection(finish).is_none(),
        "控制条盖在区域里时不透过遮罩"
    );
}

#[test]
fn bps_capture_region_with_long_screenshot_binding_replies_after_user_finishes() {
    let window = desktop_with_window(document(1000), VIEWPORT);
    let fake = window.fake.clone();
    let mut instance = ResidentInstance::new(
        fake.platform(),
        vec![HotkeyBinding {
            hotkey: Hotkey::new(Modifiers::CTRL.with(Modifiers::SHIFT), Key::PrintScreen),
            capture: CaptureRequest::function(CaptureMode::Region, DirectFunction::LongScreenshot),
        }],
    );
    assert!(instance.register_hotkeys().is_empty());
    let user = fake.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
        // 用户框选区域、按 Enter 开始，什么也不滚动，再按 Enter 完成。
        for _ in 0..3 {
            if !user.overlay_visible() {
                break;
            }
            let events = if user.sampling_region().is_none() {
                user.select_region(VIEWPORT)
            } else {
                vec![user.press_key(OverlayKey::Enter)]
            };
            for event in events {
                let _ = instance.handle(event);
            }
        }
    });

    let outcome = run_cli(["capture", "region"], &mut channel, Locale::ZhCn);

    assert_eq!(outcome.status, CliStatus::Success, "{outcome:?}");
    let copied = fake.clipboard().png_image().unwrap();
    assert_eq!(
        (copied.width(), copied.height()),
        (VIEWPORT.width, VIEWPORT.height)
    );
}

#[test]
fn selecting_moving_and_undoing_work_on_the_scaled_down_long_screenshot() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start_from_confirmation(fake);
    for offset in (0..=600).step_by(30) {
        window.scroll_to(&mut instance, offset);
    }
    feed(&mut instance, fake.click_text("完成"));
    let (view, _) = fake.overlay_canvas().expect("应显示长截屏");
    let scale = 750.0 / f64::from(view.height);
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 80 && p[2] < 80;
    let (view_x, view_y) = (view.x, view.y);

    // 在缩小的视图里画一个框（屏幕 4..20 × 4..30），再用“选择”抓住它的左边线向右下拖 (10,12)。
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(
            Point::new(view_x + 4, view_y + 4),
            Point::new(view_x + 20, view_y + 30),
        ),
    );
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.drag(
            Point::new(view_x + 5, view_y + 17),
            Point::new(view_x + 15, view_y + 29),
        ),
    );
    // 画布上也移动了，并且以缩小的比例显示。
    let (_, shown) = fake.overlay_canvas().unwrap();
    assert!(
        red(shown.pixel(15, 35)),
        "缩小的视图里，框的左边线跟着拖到了新位置"
    );
    assert!(!red(shown.pixel(5, 17)), "原位置空了");

    // 一次 Ctrl+Z 回到拖动前；再 Ctrl+Y 重做。
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    let (_, shown) = fake.overlay_canvas().unwrap();
    assert!(red(shown.pixel(5, 17)) && !red(shown.pixel(15, 35)));
    instance
        .handle(fake.press_key_with(OverlayKey::Y, Modifiers::CTRL))
        .unwrap();

    // 复制：原尺寸的长截屏上，框左边线整体移动了 (10,12) 个屏幕像素 = 乘缩小比例的像素。
    feed(&mut instance, fake.click_text("确认"));
    let copied = fake.clipboard().png_image().unwrap();
    assert_eq!((copied.width(), copied.height()), (120, 750));
    let at = |x: f64, y: f64| copied.pixel((x * scale) as u32, (y * scale) as u32);
    assert!(red(at(15.5, 35.5)), "移动后的左边线");
    assert!(!red(at(5.5, 17.5)), "原位置没有框");
}

#[test]
fn deleting_the_selection_on_the_scaled_down_long_screenshot_restores_the_stitched_image() {
    let doc = document(1000);
    let window = desktop_with_window(doc.clone(), VIEWPORT);
    let fake = &window.fake;
    let mut instance = start_from_confirmation(fake);
    for offset in (0..=600).step_by(30) {
        window.scroll_to(&mut instance, offset);
    }
    feed(&mut instance, fake.click_text("完成"));
    let (view, _) = fake.overlay_canvas().unwrap();

    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(
            Point::new(view.x + 4, view.y + 4),
            Point::new(view.x + 20, view.y + 30),
        ),
    );
    feed(&mut instance, fake.click_text("选择"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(view.x + 5, view.y + 17)),
    );
    instance.handle(fake.press_key(OverlayKey::Delete)).unwrap();
    feed(&mut instance, fake.click_text("确认"));

    let copied = fake.clipboard().png_image().unwrap();
    assert_same(&copied, &rows(&doc, 0, 750));
}
