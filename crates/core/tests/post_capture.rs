//! 截后动作：每条截图类绑定二选一。
//!
//! - 复制到剪贴板：截图完成（框选松开、单击窗口、显示器截取）后直接复制并退出截图界面；
//! - 等待确认：停留在截图界面，选区上的工具条可以标注、录屏、长截屏、OCR、翻译、另存为、
//!   确认（Enter，复制并退出）、取消（Esc，不复制）。
//!
//! 自动保存是独立的开关，与截后动作无关：截图最终交付时（复制并退出、确认）同时存盘，取消时不存。
//! 用户在遮罩上的操作由假平台模拟：工具条按钮用 `click_text`，期望图像来自 `desktop_pattern`。

use std::path::PathBuf;

use bps_core::{
    CaptureMode, CaptureRequest, Command, Config, DirectFunction, Flow, PostCaptureAction,
    ResidentInstance,
};
use bps_platform::{
    NotificationClick, OverlayKey, PlatformEvent, Point, PointerButton, Rect, StandaloneWindow,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeWindow, decode, desktop_pattern};

const SCREEN: Rect = Rect::new(0, 0, 800, 600);
/// 框选的区域：足够大，工具条的所有按钮都在显示器内。
const REGION: Rect = Rect::new(40, 30, 300, 200);

fn desk() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(SCREEN).primary())
        .with_cursor_at(Point::new(30, 20))
}

fn copy(mode: CaptureMode) -> CaptureRequest {
    CaptureRequest::new(mode, PostCaptureAction::CopyToClipboard)
}

fn await_confirmation(mode: CaptureMode) -> CaptureRequest {
    CaptureRequest::new(mode, PostCaptureAction::AwaitConfirmation)
}

fn start(fake: &FakePlatform, request: CaptureRequest) -> ResidentInstance {
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    instance.execute(Command::Capture(request)).unwrap();
    instance
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn click_button(fake: &FakePlatform, instance: &mut ResidentInstance, text: &str) {
    feed(instance, fake.click_text(text));
}

/// 区域模式 + 等待确认，已框选 [`REGION`]：选区上显示着工具条。
fn awaiting(fake: &FakePlatform, request: CaptureRequest) -> ResidentInstance {
    let mut instance = start(fake, request);
    assert!(fake.overlay_visible(), "应先显示选区遮罩");
    // 只框选（拖拽松开），不按 Enter：等待确认时选区上应出现工具条。
    feed(
        &mut instance,
        fake.drag(top_left(REGION), bottom_right(REGION)),
    );
    instance
}

fn top_left(rect: Rect) -> Point {
    Point::new(rect.x, rect.y)
}

fn bottom_right(rect: Rect) -> Point {
    Point::new(
        rect.x + rect.width as i32 - 1,
        rect.y + rect.height as i32 - 1,
    )
}

fn saved_files(fake: &FakePlatform) -> Vec<PathBuf> {
    fake.files()
}

fn pattern_png(fake: &FakePlatform) -> bps_platform::Image {
    fake.clipboard().png_image().expect("剪贴板应有 PNG 图像")
}

// ---- 复制到剪贴板：直接退出 ----

#[test]
fn copy_to_clipboard_exits_right_after_region_is_selected() {
    let fake = desk();
    let mut instance = start(&fake, copy(CaptureMode::Region));
    assert!(fake.clipboard().is_empty());

    // 只拖拽松开，不按 Enter、不双击：松开左键的那一刻就应完成。
    feed(
        &mut instance,
        fake.drag(top_left(REGION), bottom_right(REGION)),
    );

    assert!(!fake.overlay_visible(), "复制后应直接退出截图界面");
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
    assert!(saved_files(&fake).is_empty(), "没开自动保存不应存盘");
}

#[test]
fn copy_to_clipboard_does_not_finish_on_press_and_release_without_moving() {
    let fake = desk();
    let mut instance = start(&fake, copy(CaptureMode::Region));

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 100)),
    );

    assert!(fake.overlay_visible(), "没移动就松开不算框选");
    assert!(fake.clipboard().is_empty());
}

#[test]
fn copy_to_clipboard_escape_during_drag_still_cancels() {
    let fake = desk();
    let mut instance = start(&fake, copy(CaptureMode::Region));
    let mut events = fake.drag(top_left(REGION), bottom_right(REGION));
    events.truncate(3); // 按下、移动、移动：还没松开
    feed(&mut instance, events);
    assert!(fake.overlay_visible());

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
}

#[test]
fn copy_to_clipboard_exits_right_away_for_display_capture() {
    let fake = desk();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    let flow = instance
        .execute(Command::Capture(copy(CaptureMode::Display)))
        .unwrap();

    assert_eq!(flow, Flow::Continue);
    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == desktop_pattern(SCREEN));
}

#[test]
fn copy_to_clipboard_exits_after_clicking_window() {
    let window = FakeWindow::new("记事本", Rect::new(60, 40, 300, 200));
    let fake = desk().with_window(window.clone());
    let mut instance = start(&fake, copy(CaptureMode::Window));

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 80)),
    );

    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == *window.image());
}

// ---- 自动保存独立于截后动作 ----

#[test]
fn auto_save_with_copy_to_clipboard_saves_and_copies_once_delivered() {
    let fake = desk();
    let mut instance = start(&fake, copy(CaptureMode::Region).with_auto_save());

    feed(&mut instance, fake.select_region(REGION));

    let files = saved_files(&fake);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with(fake.pictures_dir().join("BetterPrtSc")));
    assert!(decode::png(&fake.file(&files[0]).unwrap()) == desktop_pattern(REGION));
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
    assert_eq!(
        fake.clipboard().files(),
        Some(files.clone()),
        "剪贴板附带存下的文件"
    );
    assert_eq!(fake.notifications().len(), 1, "应通知“截图已保存”");
    assert!(matches!(
        fake.notifications()[0].on_click,
        Some(NotificationClick::OpenFile(_))
    ));
}

#[test]
fn auto_save_is_off_by_default_even_when_awaiting_confirmation() {
    let fake = desk();
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(saved_files(&fake).is_empty());
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
}

#[test]
fn auto_save_with_await_confirmation_saves_only_on_confirm() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_auto_save(),
    );
    assert!(saved_files(&fake).is_empty(), "确认之前不应存盘");
    assert!(fake.clipboard().is_empty());

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    let files = saved_files(&fake);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(decode::png(&fake.file(&files[0]).unwrap()) == desktop_pattern(REGION));
    assert_eq!(fake.clipboard().files(), Some(files.clone()));
}

#[test]
fn cancelling_does_not_save_even_with_auto_save() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_auto_save(),
    );

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert!(saved_files(&fake).is_empty());
    assert!(fake.clipboard().is_empty());
    assert!(fake.notifications().is_empty());
}

// ---- 等待确认 ----

#[test]
fn await_confirmation_stays_on_screen_with_toolbar_until_decided() {
    let fake = desk();
    let _instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    assert!(fake.overlay_visible(), "框选松开后应停留在截图界面");
    assert!(fake.clipboard().is_empty(), "确认之前不应复制");
    let scene = fake.overlay_scene().unwrap();
    for text in [
        "矩形",
        "箭头",
        "马赛克",
        "文字",
        "录屏",
        "长截屏",
        "OCR",
        "翻译",
        "另存为",
        "确认",
        "取消",
    ] {
        assert!(scene.text_rect(text).is_some(), "工具条上应有“{text}”");
    }
}

#[test]
fn enter_confirms_by_copying_and_exiting() {
    let fake = desk();
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
}

#[test]
fn confirm_button_copies_annotated_screenshot_and_exits() {
    let fake = desk();
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));
    click_button(&fake, &mut instance, "矩形");
    feed(
        &mut instance,
        fake.drag(Point::new(80, 60), Point::new(200, 150)),
    );

    click_button(&fake, &mut instance, "确认");

    assert!(!fake.overlay_visible());
    let copied = pattern_png(&fake);
    assert_eq!(
        (copied.width(), copied.height()),
        (REGION.width, REGION.height)
    );
    assert!(
        copied != desktop_pattern(REGION),
        "复制的应是画了标注的截图"
    );
}

#[test]
fn escape_and_cancel_button_exit_without_copying() {
    for cancel in [
        (|fake: &FakePlatform, instance: &mut ResidentInstance| {
            feed(instance, vec![fake.press_key(OverlayKey::Escape)])
        }) as fn(&FakePlatform, &mut ResidentInstance),
        |fake, instance| click_button(fake, instance, "取消"),
    ] {
        let fake = desk();
        let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

        cancel(&fake, &mut instance);

        assert!(!fake.overlay_visible());
        assert!(fake.clipboard().is_empty(), "取消不应复制");
        assert!(saved_files(&fake).is_empty());
    }
}

#[test]
fn display_mode_enters_await_confirmation_with_whole_display_selected() {
    let fake = desk();
    let mut instance = start(&fake, await_confirmation(CaptureMode::Display));

    assert!(fake.overlay_visible(), "显示器模式也应停留在截图界面");
    assert!(fake.overlay_scene().unwrap().text_rect("确认").is_some());
    assert!(fake.clipboard().is_empty());

    feed(&mut instance, vec![fake.press_key(OverlayKey::Enter)]);

    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == desktop_pattern(SCREEN));
}

#[test]
fn window_mode_enters_await_confirmation_after_clicking_window() {
    let window = FakeWindow::new("记事本", Rect::new(60, 40, 300, 200));
    let fake = desk().with_window(window.clone());
    let mut instance = start(&fake, await_confirmation(CaptureMode::Window));
    assert!(fake.clipboard().is_empty());

    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 80)),
    );

    assert!(fake.overlay_visible(), "单击窗口后应停留在截图界面");
    assert!(fake.clipboard().is_empty(), "确认之前不应复制");
    for text in [
        "矩形",
        "录屏",
        "长截屏",
        "OCR",
        "翻译",
        "另存为",
        "确认",
        "取消",
    ] {
        assert!(
            fake.overlay_scene().unwrap().text_rect(text).is_some(),
            "工具条上应有“{text}”"
        );
    }

    click_button(&fake, &mut instance, "确认");

    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == *window.image());
}

#[test]
fn escape_cancels_window_await_confirmation_without_copying() {
    let fake = desk().with_window(FakeWindow::new("记事本", Rect::new(60, 40, 300, 200)));
    let mut instance = start(
        &fake,
        await_confirmation(CaptureMode::Window).with_auto_save(),
    );
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 80)),
    );

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
    assert!(saved_files(&fake).is_empty());
}

// ---- 从等待确认进入其他功能 ----

#[test]
fn save_as_writes_the_chosen_file_and_exits_without_copying() {
    let fake = desk().with_dir(r"D:\Shots");
    let target = PathBuf::from(r"D:\Shots\mine.png");
    fake.answer_save_as(Some(target.clone()));
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    click_button(&fake, &mut instance, "另存为");

    assert!(!fake.overlay_visible());
    assert!(decode::png(&fake.file(&target).unwrap()) == desktop_pattern(REGION));
    assert!(fake.clipboard().is_empty(), "另存为不复制");
}

#[test]
fn cancelling_the_save_as_dialog_returns_to_the_toolbar() {
    let fake = desk();
    fake.answer_save_as(None);
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    click_button(&fake, &mut instance, "另存为");

    assert!(fake.overlay_visible());
    assert!(fake.clipboard().is_empty());
}

#[test]
fn ocr_button_opens_result_panel_with_the_selection_and_exits() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_auto_save(),
    );

    click_button(&fake, &mut instance, "OCR");

    assert!(!fake.overlay_visible());
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(REGION),
            translate: false,
            auto_copy: false,
        }]
    );
    assert!(fake.clipboard().is_empty(), "OCR 不复制");
    assert!(saved_files(&fake).is_empty(), "OCR 不自动保存");
}

#[test]
fn toolbar_ocr_still_opens_result_panel_when_auto_copy_is_on() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_ocr_auto_copy(),
    );

    click_button(&fake, &mut instance, "OCR");

    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(REGION),
            translate: false,
            auto_copy: false,
        }],
        "“识别后自动复制”只影响直达 OCR"
    );
}

#[test]
fn translate_button_opens_result_panel_that_translates_after_recognition() {
    let fake = desk();
    let mut instance = awaiting(&fake, await_confirmation(CaptureMode::Region));

    click_button(&fake, &mut instance, "翻译");

    assert!(!fake.overlay_visible());
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(REGION),
            translate: true,
            auto_copy: false,
        }]
    );
    assert!(fake.http_requests().is_empty(), "打开面板时不联网");
}

#[test]
fn ocr_of_window_await_confirmation_uses_the_window_content() {
    let window = FakeWindow::new("记事本", Rect::new(60, 40, 300, 200));
    let fake = desk().with_window(window.clone());
    let mut instance = start(&fake, await_confirmation(CaptureMode::Window));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 80)),
    );

    click_button(&fake, &mut instance, "OCR");

    assert!(!fake.overlay_visible());
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: window.image().clone(),
            translate: false,
            auto_copy: false,
        }]
    );
}

#[test]
fn record_button_starts_recording_the_selection_and_same_hotkey_press_stops_it() {
    let fake = desk();
    let request = await_confirmation(CaptureMode::Region);
    let mut instance = awaiting(&fake, request.clone());

    click_button(&fake, &mut instance, "录屏");

    assert!(!fake.overlay_visible(), "开始录制后遮罩应关闭");
    assert!(fake.capturing_video(), "应以当前选区开始录制");
    assert!(fake.clipboard().is_empty());

    let period = fake.ticker_period().expect("录制期间节拍应在运行");
    fake.advance_clock(period * 3);
    instance.handle(fake.tick().unwrap()).unwrap();
    // 再次执行同一个捕获请求（再按一次同一个热键）停止录屏并存储。
    instance.execute(Command::Capture(request)).unwrap();

    assert!(!fake.capturing_video());
    let files = saved_files(&fake);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].extension().and_then(|e| e.to_str()), Some("mp4"));
    assert_eq!(fake.clipboard().files(), Some(files.clone()));
}

#[test]
fn record_button_in_window_mode_records_that_window() {
    let window = FakeWindow::new("记事本", Rect::new(60, 40, 300, 200));
    let fake = desk().with_window(window);
    let mut instance = start(&fake, await_confirmation(CaptureMode::Window));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 80)),
    );

    click_button(&fake, &mut instance, "录屏");

    assert!(!fake.overlay_visible());
    assert!(fake.capturing_video());
}

#[test]
fn long_screenshot_button_starts_sampling_the_selection_then_returns_to_confirmation() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_auto_save(),
    );

    click_button(&fake, &mut instance, "长截屏");

    assert!(fake.overlay_visible());
    assert_eq!(fake.sampling_region(), Some(REGION), "应以当前选区开始采样");
    assert!(fake.clipboard().is_empty());

    click_button(&fake, &mut instance, "完成");

    assert_eq!(fake.sampling_region(), None, "完成后停止采样");
    assert!(fake.overlay_visible(), "拼好后应回到等待确认");
    assert!(fake.clipboard().is_empty());
    assert!(saved_files(&fake).is_empty());

    click_button(&fake, &mut instance, "确认");

    assert!(!fake.overlay_visible());
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
    assert_eq!(saved_files(&fake).len(), 1, "确认时按自动保存存盘");
}

#[test]
fn escape_during_long_screenshot_from_confirmation_cancels_everything() {
    let fake = desk();
    let mut instance = awaiting(
        &fake,
        await_confirmation(CaptureMode::Region).with_auto_save(),
    );
    click_button(&fake, &mut instance, "长截屏");

    feed(&mut instance, vec![fake.press_key(OverlayKey::Escape)]);

    assert!(!fake.overlay_visible());
    assert_eq!(fake.sampling_region(), None);
    assert!(fake.clipboard().is_empty());
    assert!(saved_files(&fake).is_empty());
}

// ---- 直达功能不变 ----

#[test]
fn direct_functions_still_go_straight_into_their_feature() {
    // OCR：框选后直接打开结果面板，不复制。
    let fake = desk();
    let mut instance = start(
        &fake,
        CaptureRequest::function(CaptureMode::Region, DirectFunction::Ocr),
    );
    feed(&mut instance, fake.select_region(REGION));
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(REGION),
            translate: false,
            auto_copy: false,
        }]
    );
    assert!(fake.clipboard().is_empty());

    // 录屏：在实时画面上框选，框选完成后开始录制。
    let fake = desk();
    let mut instance = start(
        &fake,
        CaptureRequest::function(CaptureMode::Region, DirectFunction::Record),
    );
    feed(&mut instance, fake.select_region(REGION));
    assert!(fake.capturing_video());

    // 长截屏：框选后按“开始”才开始采样。
    let fake = desk();
    let mut instance = start(
        &fake,
        CaptureRequest::function(CaptureMode::Region, DirectFunction::LongScreenshot),
    );
    feed(
        &mut instance,
        fake.drag(top_left(REGION), bottom_right(REGION)),
    );
    assert_eq!(fake.sampling_region(), None);
    click_button(&fake, &mut instance, "开始");
    assert_eq!(fake.sampling_region(), Some(REGION));
    click_button(&fake, &mut instance, "完成");
    assert!(!fake.overlay_visible(), "直达的长截屏完成后直接复制并退出");
    assert!(pattern_png(&fake) == desktop_pattern(REGION));
}

// ---- 配置 ----

fn parse(text: &str) -> Result<Config, String> {
    Config::parse(text).map_err(|e| e.message())
}

#[test]
fn bindings_choose_after_or_function_and_auto_save_independently() {
    let config = parse(
        r#"
[[bindings]]
hotkey = "F9"
mode = "region"
after = "await_confirmation"
auto_save = true

[[bindings]]
hotkey = "F10"
mode = "window"
after = "copy_to_clipboard"

[[bindings]]
hotkey = "F11"
mode = "region"
function = "long_screenshot"
auto_save = true

[[bindings]]
hotkey = "F12"
mode = "display"
"#,
    )
    .unwrap();
    let requests: Vec<_> = config.bindings.iter().map(|b| b.capture.clone()).collect();

    assert_eq!(
        requests[0],
        await_confirmation(CaptureMode::Region).with_auto_save()
    );
    assert_eq!(requests[1], copy(CaptureMode::Window));
    assert_eq!(
        requests[2],
        CaptureRequest::function(CaptureMode::Region, DirectFunction::LongScreenshot)
            .with_auto_save()
    );
    assert_eq!(
        requests[3],
        copy(CaptureMode::Display),
        "缺省为复制到剪贴板"
    );
}

#[test]
fn default_bindings_follow_the_new_model() {
    let kinds: Vec<_> = Config::default()
        .bindings
        .iter()
        .map(|b| {
            (
                b.hotkey.to_string(),
                b.capture.mode,
                b.capture.kind,
                b.capture.auto_save,
            )
        })
        .collect();
    use PostCaptureAction::{AwaitConfirmation, CopyToClipboard};
    use bps_core::CaptureKind::{Function, Screenshot};
    assert_eq!(
        kinds,
        [
            (
                "PrintScreen".to_owned(),
                CaptureMode::Region,
                Screenshot(AwaitConfirmation),
                false
            ),
            (
                "Shift+PrintScreen".to_owned(),
                CaptureMode::Display,
                Screenshot(CopyToClipboard),
                true
            ),
            (
                "Ctrl+PrintScreen".to_owned(),
                CaptureMode::Window,
                Screenshot(AwaitConfirmation),
                false
            ),
            (
                "Alt+PrintScreen".to_owned(),
                CaptureMode::Region,
                Function(DirectFunction::Record),
                false
            ),
            (
                "Ctrl+Shift+PrintScreen".to_owned(),
                CaptureMode::Region,
                Function(DirectFunction::LongScreenshot),
                false
            ),
            (
                "Win+Shift+O".to_owned(),
                CaptureMode::Region,
                Function(DirectFunction::Ocr),
                false
            ),
        ]
    );
}

#[test]
fn legacy_actions_key_is_rejected_with_a_clear_error() {
    let error = Config::parse(
        "[[bindings]]\nhotkey = \"F9\"\nmode = \"region\"\nactions = [\"copy_to_clipboard\", \"auto_save\"]\n",
    )
    .unwrap_err();

    assert_eq!(error.line(), Some(4));
    let message = error.message();
    assert!(message.contains("actions"), "{message}");
    assert!(message.contains("after"), "应指出新写法：{message}");
    assert!(message.contains("auto_save"), "应指出新写法：{message}");
}

#[test]
fn legacy_actions_in_config_file_notify_and_fall_back_to_defaults() {
    let fake = desk().with_config_file(
        "[[bindings]]\nhotkey = \"F9\"\nmode = \"region\"\nactions = [\"annotate\"]\n",
    );

    let _instance = ResidentInstance::start(fake.platform());

    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert!(
        notifications[0].body.contains("actions"),
        "{notifications:?}"
    );
    assert!(
        fake.registered_hotkeys()
            .contains(&"PrintScreen".parse().unwrap()),
        "应回退到默认配置"
    );
}

#[test]
fn invalid_after_function_and_auto_save_combinations_are_rejected() {
    let cases = [
        ("after = \"annotate\"", "annotate"),
        ("function = \"translate\"", "translate"),
        (
            "after = \"copy_to_clipboard\"\nfunction = \"ocr\"",
            "function",
        ),
        ("function = \"record\"\nauto_save = true", "record"),
        ("function = \"ocr\"\nauto_save = true", "ocr"),
    ];
    for (body, mentions) in cases {
        let text = format!("[[bindings]]\nhotkey = \"F9\"\nmode = \"region\"\n{body}\n");
        let error = Config::parse(&text).unwrap_err();
        assert!(
            error.message().contains(mentions),
            "{body}: {}",
            error.message()
        );
        assert!(error.line().is_some());
    }
}
