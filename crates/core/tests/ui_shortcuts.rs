use bps_core::{Config, SettingsDocument};

#[test]
fn ui_shortcuts_round_trip_and_reject_conflicts() {
    let document =
        SettingsDocument::parse("ocr_shortcut = 'Ctrl+F8'\ntranslate_shortcut = 'Alt+T'\n")
            .unwrap();
    let mut settings = document.settings().clone();
    assert_eq!(settings.ocr_shortcut.unwrap().to_string(), "Ctrl+F8");
    settings.ocr_shortcut = Some("Ctrl+F9".parse().unwrap());
    let text = document.render(&settings).unwrap();
    assert_eq!(
        Config::parse(&text)
            .unwrap()
            .ocr_shortcut
            .unwrap()
            .to_string(),
        "Ctrl+F9"
    );
    for text in [
        "ocr_shortcut = 'Ctrl+Z'",
        "ocr_shortcut = 'Ctrl+Win+Shift+Y'",
        "ocr_shortcut = 'Ctrl+T'",
        "ocr_shortcut = 'Ctrl+F8'\n[[bindings]]\nhotkey = 'Ctrl+F8'\nmode = 'region'",
    ] {
        assert!(Config::parse(text).is_err(), "{text}");
    }
}

#[test]
fn translate_copy_writes_only_translation_and_preserves_clipboard_on_failure() {
    use bps_core::{Locale, OcrCopy, copy_translated_text};
    use bps_platform::{ClipboardItem, Image};
    use bps_platform_fake::FakePlatform;
    let config = Config::parse("[translation]\nprovider='deepl'\napi_key='test:fx'").unwrap();
    for (status, reply, success) in [
        (200, r#"{"translations":[{"text":"Hello"}]}"#, true),
        (200, r#"{"translations":[{"text":"  "}]}"#, false),
        (503, "unavailable", false),
    ] {
        let fake = FakePlatform::new()
            .with_recognition_language("en", "English")
            .with_default_recognition_language("en")
            .with_recognized_text("en", &[&["original"]])
            .with_http_reply(status, reply);
        let mut platform = fake.platform();
        platform
            .clipboard
            .write(vec![ClipboardItem::Text("previous".into())])
            .unwrap();
        let result = copy_translated_text(
            Image::from_rgba(1, 1, vec![0; 4]),
            fake.text_recognizer().as_ref(),
            platform.clipboard.as_mut(),
            config.translation.as_ref(),
            Locale::ZhCn,
            fake.http_transport().as_ref(),
        );
        if success {
            assert_eq!(result, OcrCopy::Copied { characters: 5 });
        } else {
            assert!(matches!(result, OcrCopy::Failed(_)));
        }
        assert_eq!(
            fake.clipboard().items(),
            [ClipboardItem::Text(
                if success { "Hello" } else { "previous" }.into()
            )]
        );
    }
}

#[test]
fn shortcuts_dispatch_before_and_after_lock_but_not_during_text_input() {
    use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
    use bps_platform::{Key, Modifiers, OverlayKey, Point, Rect, StandaloneWindow};
    use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};
    let region = Rect::new(40, 30, 300, 200);
    for locked in [false, true] {
        for translate in [false, true] {
            for copy in [false, true] {
                let fake = FakePlatform::new()
                    .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
                let mut instance = ResidentInstance::new(fake.platform(), vec![]);
                let mut request =
                    CaptureRequest::new(CaptureMode::Region, PostCaptureAction::AwaitConfirmation);
                request.ocr_auto_copy = copy;
                instance.execute(Command::Capture(request)).unwrap();
                for event in fake.drag(Point::new(40, 30), Point::new(339, 229)) {
                    instance.handle(event).unwrap();
                }
                if locked {
                    for event in fake.click_text("矩形") {
                        instance.handle(event).unwrap();
                    }
                }
                instance
                    .handle(fake.press_key_with(
                        OverlayKey::Shortcut(Key::Letter(if translate { 'T' } else { 'O' })),
                        Modifiers::CTRL,
                    ))
                    .unwrap();
                assert_eq!(
                    fake.opened_windows(),
                    [StandaloneWindow::ResultPanel {
                        screenshot: desktop_pattern(region),
                        translate,
                        auto_copy: copy
                    }]
                );
            }
        }
    }
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    for event in fake.drag(Point::new(40, 30), Point::new(339, 229)) {
        instance.handle(event).unwrap();
    }
    for event in fake.click_text("文字") {
        instance.handle(event).unwrap();
    }
    for event in fake.click(bps_platform::PointerButton::Left, Point::new(100, 100)) {
        instance.handle(event).unwrap();
    }
    instance
        .handle(fake.press_key_with(OverlayKey::Shortcut(Key::Letter('O')), Modifiers::CTRL))
        .unwrap();
    instance
        .handle(fake.press_key_with(OverlayKey::Shortcut(Key::Letter('T')), Modifiers::CTRL))
        .unwrap();
    assert!(fake.opened_windows().is_empty());
    assert!(fake.overlay_visible());
}

#[test]
fn legacy_conflict_preserves_config_and_custom_keys_reload_with_tooltips() {
    use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
    use bps_platform::{Key, Modifiers, OverlayInput, OverlayKey, Point, Rect};
    use bps_platform_fake::{FakeDisplay, FakePlatform};
    let old = "# existing config\n[[bindings]]\nhotkey='Ctrl+O'\nmode='region'";
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_config_file(old);
    let mut instance = ResidentInstance::start(fake.platform());
    assert_eq!(instance.config().ocr_shortcut, None);
    assert_eq!(instance.config().bindings[0].hotkey.to_string(), "Ctrl+O");
    assert_eq!(fake.config_file_text().as_deref(), Some(old));
    assert!(
        fake.notifications()
            .iter()
            .any(|n| n.body.contains("Ctrl+O"))
    );
    let changed = format!("ocr_shortcut='Alt+F8'\ntranslate_shortcut='Ctrl+F9'\n{old}");
    instance
        .handle(fake.edit_config_file(&changed).unwrap())
        .unwrap();
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    for event in fake.drag(Point::new(40, 30), Point::new(339, 229)) {
        instance.handle(event).unwrap();
    }
    let button = fake.overlay_scene().unwrap().text_rect("OCR").unwrap();
    instance
        .handle(fake.overlay_input(OverlayInput::PointerMoved {
            position: Point::new(button.x + 2, button.y + 2),
            modifiers: Modifiers::NONE,
        }))
        .unwrap();
    assert!(
        fake.overlay_scene()
            .unwrap()
            .labels()
            .any(|l| l.text.contains("Alt+F8"))
    );
    instance
        .handle(fake.press_key_with(OverlayKey::Shortcut(Key::F(8)), Modifiers::ALT))
        .unwrap();
    assert_eq!(fake.opened_windows().len(), 1);
    assert!(!fake.overlay_visible());
}

#[test]
fn translation_without_service_or_with_timeout_keeps_existing_text() {
    use bps_core::{Locale, OcrCopy, copy_translated_text};
    use bps_platform::{ClipboardItem, Image};
    use bps_platform_fake::FakePlatform;
    let config = Config::parse("[translation]\nprovider='deepl'\napi_key='secret:fx'").unwrap();
    for service in [None, config.translation.as_ref()] {
        let fake = FakePlatform::new()
            .with_recognition_language("en", "English")
            .with_default_recognition_language("en")
            .with_recognized_text("en", &[&["original"]])
            .with_http_failure("timeout secret:fx");
        let mut platform = fake.platform();
        platform
            .clipboard
            .write(vec![ClipboardItem::Text("previous".into())])
            .unwrap();
        let result = copy_translated_text(
            Image::from_rgba(1, 1, vec![0; 4]),
            fake.text_recognizer().as_ref(),
            platform.clipboard.as_mut(),
            service,
            Locale::ZhCn,
            fake.http_transport().as_ref(),
        );
        let OcrCopy::Failed(message) = result else {
            panic!("expected failure")
        };
        assert!(!message.contains("secret:fx"));
        if service.is_none() {
            assert!(message.contains("config.toml"));
        }
        assert_eq!(
            fake.clipboard().items(),
            [ClipboardItem::Text("previous".into())]
        );
        assert!(fake.opened_windows().is_empty());
    }
}
