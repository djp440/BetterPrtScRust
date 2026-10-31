use bps_core::{CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance};
use bps_platform::{Modifiers, OverlayKey, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, desktop_pattern};

fn feed(instance: &mut ResidentInstance, events: Vec<bps_platform::PlatformEvent>) {
    for event in events {
        instance.handle(event).unwrap();
    }
}

#[test]
fn resize_before_first_annotation_is_one_undoable_step() {
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 229)),
    );
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(60, 50)),
    );
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(
        fake.overlay_canvas().unwrap(),
        (
            Rect::new(40, 30, 300, 200),
            desktop_pattern(Rect::new(40, 30, 300, 200))
        )
    );
}

#[test]
fn resizing_after_annotation_keeps_screen_position_and_undo_order() {
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 229)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(90, 80), Point::new(160, 150)),
    );
    let original = fake.overlay_canvas().unwrap().1;
    // 左上角向右下缩小，标注的屏幕位置保持不动。
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(60, 50)),
    );
    let (region, image) = fake.overlay_canvas().unwrap();
    assert_eq!(region, Rect::new(60, 50, 280, 180));
    assert_eq!(image.pixel(30, 30), [240, 40, 40, 255]);
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(
        fake.overlay_canvas().unwrap(),
        (Rect::new(40, 30, 300, 200), original)
    );
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(
        fake.overlay_canvas().unwrap().1,
        desktop_pattern(Rect::new(40, 30, 300, 200))
    );
    instance
        .handle(fake.press_key_with(OverlayKey::Y, Modifiers::CTRL))
        .unwrap();
    instance
        .handle(fake.press_key_with(OverlayKey::Y, Modifiers::CTRL))
        .unwrap();
    assert_eq!(fake.overlay_canvas().unwrap(), (region, image));
}

#[test]
fn clipped_annotations_return_when_expanding_and_resize_never_draws_a_shape() {
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 229)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(90, 80), Point::new(160, 150)),
    );
    let before = fake.overlay_canvas().unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(200, 170)),
    );
    assert_eq!(
        fake.overlay_canvas().unwrap().1,
        desktop_pattern(Rect::new(200, 170, 140, 60))
    );
    feed(
        &mut instance,
        fake.drag(Point::new(200, 170), Point::new(40, 30)),
    );
    assert_eq!(fake.overlay_canvas().unwrap(), before);
    feed(&mut instance, fake.click_text("确认"));
    assert_eq!(fake.clipboard().png_image().unwrap(), before.1);
}

#[test]
fn window_resize_undo_redo_restores_image_title_and_recording_target() {
    use bps_platform::PointerButton;
    use bps_platform_fake::{FakeWindow, decode};
    for undo in [false, true] {
        for redo in [false, true] {
            let window = FakeWindow::new("原窗口", Rect::new(60, 40, 300, 200));
            let fake = FakePlatform::new()
                .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
                .with_window(window.clone());
            let mut instance = ResidentInstance::new(fake.platform(), vec![]);
            let mut request =
                CaptureRequest::new(CaptureMode::Region, PostCaptureAction::AwaitConfirmation);
            request.output.filename_template =
                bps_core::FilenameTemplate::parse("shot-{window_title}").unwrap();
            request.auto_save = true;
            instance.execute(Command::Capture(request)).unwrap();
            feed(
                &mut instance,
                fake.click(PointerButton::Left, Point::new(100, 100)),
            );
            feed(
                &mut instance,
                fake.drag(Point::new(359, 140), Point::new(319, 140)),
            );
            if undo {
                instance
                    .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
                    .unwrap();
            }
            if redo {
                instance
                    .handle(fake.press_key_with(OverlayKey::Y, Modifiers::CTRL))
                    .unwrap();
            }
            let restored = undo && !redo;
            let expected = if restored {
                window.image().clone()
            } else {
                desktop_pattern(Rect::new(60, 40, 260, 200))
            };
            assert_eq!(fake.overlay_canvas().unwrap().1, expected);
            feed(&mut instance, fake.click_text("录屏"));
            fake.advance_clock(std::time::Duration::from_millis(100));
            instance.handle(fake.tick().unwrap()).unwrap();
            feed(&mut instance, fake.click_recording_toolbar("停止"));
            let path = fake.clipboard().files().unwrap()[0].clone();
            let video = decode::video(&fake.file(&path).unwrap());
            assert!(!video.frames.is_empty());
            assert_eq!(video.frames[0].image, expected);
            assert_eq!(path.to_string_lossy().contains("原窗口"), restored);
        }
    }
}

#[test]
fn locked_resize_snaps_only_the_dragged_edge() {
    use bps_platform_fake::FakeWindow;
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary())
        .with_window(FakeWindow::new("吸附目标", Rect::new(400, 100, 200, 200)));
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 329)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(339, 180), Point::new(395, 180)),
    );
    assert_eq!(
        fake.overlay_canvas().unwrap().0,
        Rect::new(40, 30, 360, 300)
    );
}

#[test]
fn dragging_handle_commits_text_before_the_resize_step() {
    use bps_platform::PointerButton;
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 229)),
    );
    feed(&mut instance, fake.click_text("文字"));
    feed(
        &mut instance,
        fake.click(PointerButton::Left, Point::new(100, 100)),
    );
    feed(&mut instance, fake.type_text("测试"));
    assert!(fake.overlay_scene().unwrap().text_caret.is_some());
    let original = fake.overlay_canvas().unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(339, 130), Point::new(399, 130)),
    );
    assert!(fake.overlay_scene().unwrap().text_caret.is_none());
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(fake.overlay_canvas().unwrap(), original);
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(
        fake.overlay_canvas().unwrap().1,
        desktop_pattern(Rect::new(40, 30, 300, 200))
    );
}

#[test]
fn locked_handles_flip_and_clamp_on_a_scaled_negative_origin_display() {
    let fake = FakePlatform::new().with_display(
        FakeDisplay::new(Rect::new(-800, 0, 800, 600))
            .primary()
            .scale_factor(1.5),
    );
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(-600, 80), Point::new(-301, 279)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(-600, 180), Point::new(-250, 180)),
    );
    assert_eq!(
        fake.overlay_canvas().unwrap().0,
        Rect::new(-301, 80, 52, 200)
    );
    feed(
        &mut instance,
        fake.drag(Point::new(-250, 180), Point::new(50, 180)),
    );
    assert_eq!(
        fake.overlay_canvas().unwrap().0,
        Rect::new(-301, 80, 301, 200)
    );
}

#[test]
fn double_clicking_a_handle_does_not_block_history() {
    let fake =
        FakePlatform::new().with_display(FakeDisplay::new(Rect::new(0, 0, 800, 600)).primary());
    let mut instance = ResidentInstance::new(fake.platform(), vec![]);
    instance
        .execute(Command::Capture(CaptureRequest::new(
            CaptureMode::Region,
            PostCaptureAction::AwaitConfirmation,
        )))
        .unwrap();
    feed(
        &mut instance,
        fake.drag(Point::new(40, 30), Point::new(339, 229)),
    );
    feed(&mut instance, fake.click_text("矩形"));
    feed(
        &mut instance,
        fake.drag(Point::new(90, 80), Point::new(160, 150)),
    );
    feed(&mut instance, fake.double_click(Point::new(40, 30)));
    instance
        .handle(fake.press_key_with(OverlayKey::Z, Modifiers::CTRL))
        .unwrap();
    assert_eq!(
        fake.overlay_canvas().unwrap().1,
        desktop_pattern(Rect::new(40, 30, 300, 200))
    );
}
