//! OCR 直达功能：截图交给结果面板，由结果面板在本机识别文字。
//!
//! 常驻实例只负责打开结果面板并把截图交给它（`StandaloneWindow::ResultPanel`），
//! 识别本身在结果面板里进行，见 `result_panel.rs`。

use std::cell::RefCell;
use std::rc::Rc;

use bps_core::{
    CaptureMode, CaptureRequest, Config, DirectFunction, HotkeyBinding, OcrCopy, Reply, Request,
    ResidentInstance, SettingsDocument, WhenNotRunning, copy_recognized_text, forward,
    set_ocr_auto_copy,
};
use bps_platform::{
    ClipboardItem, Hotkey, Notification, NotificationClick, PlatformEvent, Point, Rect,
    StandaloneWindow,
};
use bps_platform_fake::{FakeDisplay, FakePlatform, FakeResidentChannel, desktop_pattern};

fn single_display() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
        .with_cursor_at(Point::new(30, 20))
}

fn feed(instance: &mut ResidentInstance, events: Vec<PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

fn hotkey(text: &str) -> Hotkey {
    text.parse().unwrap()
}

#[test]
fn default_ocr_hotkey_opens_result_panel_for_selected_region() {
    let fake = single_display();
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .handle(fake.press(hotkey("Win+Shift+O")).unwrap())
        .unwrap();
    let region = Rect::new(10, 5, 20, 12);
    feed(&mut instance, fake.select_region(region));

    assert!(!fake.overlay_visible());
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region),
            translate: false,
            auto_copy: false,
        }]
    );
    assert!(fake.clipboard().is_empty(), "只做 OCR 时不应写剪贴板");
    assert!(
        fake.update_check_requests().is_empty(),
        "OCR 不发出任何网络请求"
    );
    assert!(
        fake.notifications().is_empty(),
        "{:?}",
        fake.notifications()
    );
}

#[test]
fn releasing_the_drag_starts_ocr_without_confirming() {
    let fake = single_display();
    let mut instance = instance_with_ocr_binding(&fake);

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(10, 5, 20, 12);
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    assert!(!fake.overlay_visible(), "松开即开始 OCR，不等 Enter");
    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region),
            translate: false,
            auto_copy: false,
        }]
    );
}

/// 只有一条热键绑定（Ctrl+F9 → 区域 OCR）的常驻实例。
fn instance_with_ocr_binding(fake: &FakePlatform) -> ResidentInstance {
    let binding = HotkeyBinding {
        hotkey: hotkey("Ctrl+F9"),
        capture: CaptureRequest::function(CaptureMode::Region, DirectFunction::Ocr),
    };
    let mut instance = ResidentInstance::new(fake.platform(), vec![binding]);
    let _ = instance.register_hotkeys();
    instance
}

#[test]
fn result_panel_that_cannot_be_opened_is_reported() {
    let fake = single_display().with_window_launch_failing();
    let mut instance = instance_with_ocr_binding(&fake);

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let mut events = fake.drag(Point::new(3, 4), Point::new(32, 12));
    let release = events.pop().unwrap();
    feed(&mut instance, events);
    let result = instance.handle(release);

    assert!(result.is_err());
    assert!(fake.clipboard().is_empty(), "OCR 不复制");
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "无法打开结果面板");
    assert!(
        notifications[0].body.contains("找不到程序文件"),
        "{notifications:?}"
    );
}

// —— 识别后自动复制，不弹面板 ——

const AUTO_COPY_CONFIG: &str = r#"
ocr_auto_copy = true

[[bindings]]
hotkey = "Ctrl+F9"
mode = "region"
function = "ocr"
"#;

#[test]
fn auto_copy_setting_opens_result_panel_without_ui() {
    let fake = single_display().with_config_file(AUTO_COPY_CONFIG);
    let mut instance = ResidentInstance::start(fake.platform());

    instance
        .handle(fake.press(hotkey("Ctrl+F9")).unwrap())
        .unwrap();
    let region = Rect::new(10, 5, 20, 12);
    feed(
        &mut instance,
        fake.drag(Point::new(10, 5), Point::new(29, 16)),
    );

    assert_eq!(
        fake.opened_windows(),
        [StandaloneWindow::ResultPanel {
            screenshot: desktop_pattern(region),
            translate: false,
            auto_copy: true,
        }]
    );
    assert!(fake.clipboard().is_empty(), "复制在结果面板子进程里完成");
}

/// 系统装有中文 OCR 语言包、识别结果为 `lines` 的假平台。
fn recognizing(lines: &[&[&str]]) -> FakePlatform {
    single_display()
        .with_recognition_language("zh-Hans-CN", "中文(中华人民共和国)")
        .with_default_recognition_language("zh-Hans-CN")
        .with_recognized_text("zh-Hans-CN", lines)
}

fn copy_text(fake: &FakePlatform) -> OcrCopy {
    let mut platform = fake.platform();
    copy_recognized_text(
        desktop_pattern(Rect::new(0, 0, 8, 8)),
        fake.text_recognizer().as_ref(),
        platform.clipboard.as_mut(),
    )
}

#[test]
fn recognized_text_is_copied_to_the_clipboard() {
    let fake = recognizing(&[&["Hello", "World"], &["第二行"]]);

    let copy = copy_text(&fake);

    assert_eq!(copy, OcrCopy::Copied { characters: 13 });
    assert_eq!(
        fake.clipboard().items(),
        [ClipboardItem::Text("Hello World\n第二行".to_owned())]
    );
}

#[test]
fn nothing_recognized_leaves_the_clipboard_alone() {
    let fake = recognizing(&[]);

    assert_eq!(copy_text(&fake), OcrCopy::Empty);
    assert!(fake.clipboard().is_empty());
}

#[test]
fn missing_language_pack_leaves_the_clipboard_alone() {
    let fake = single_display();

    assert_eq!(copy_text(&fake), OcrCopy::NoLanguagePack);
    assert!(fake.clipboard().is_empty());
}

fn report(copy: OcrCopy) -> Vec<Notification> {
    let fake = single_display();
    let instance = Rc::new(RefCell::new(ResidentInstance::start(fake.platform())));
    let inside = instance.clone();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = inside.borrow_mut().handle(event);
    });
    let request = Request::OcrCopied(copy);
    assert_eq!(Request::decode(&request.encode()), Some(request.clone()));
    assert_eq!(
        forward(&request, &mut channel, WhenNotRunning::GiveUp).unwrap(),
        Reply::Done
    );
    fake.notifications()
}

#[test]
fn copied_text_is_reported_with_its_length() {
    let notifications = report(OcrCopy::Copied { characters: 13 });

    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "已复制 13 个字");
}

#[test]
fn empty_result_is_reported() {
    let notifications = report(OcrCopy::Empty);

    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "没有识别到文字");
}

#[test]
fn missing_language_pack_is_reported_with_a_way_to_install_one() {
    let notifications = report(OcrCopy::NoLanguagePack);

    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(
        notifications[0].on_click,
        Some(NotificationClick::OpenOcrLanguageSettings)
    );
}

#[test]
fn recognition_failure_is_reported_with_its_reason() {
    let notifications = report(OcrCopy::Failed("引擎出错".to_owned()));

    assert_eq!(notifications[0].body, "引擎出错");
}

#[test]
fn toggling_auto_copy_writes_it_back_to_the_config_text() {
    let text = "language = \"zh-CN\"\n\n[[bindings]]\nhotkey = \"Ctrl+F9\"\nmode = \"region\"\nfunction = \"ocr\"\n";

    let on = set_ocr_auto_copy(text, true).unwrap();
    assert!(Config::parse(&on).unwrap().ocr_auto_copy);
    assert!(
        Config::parse(&on).unwrap().bindings[0]
            .capture
            .ocr_auto_copy
    );
    assert!(
        SettingsDocument::parse(&on)
            .unwrap()
            .settings()
            .ocr_auto_copy
    );
    assert!(on.contains("language = \"zh-CN\""));

    let off = set_ocr_auto_copy(&on, false).unwrap();
    assert!(!Config::parse(&off).unwrap().ocr_auto_copy);
}
