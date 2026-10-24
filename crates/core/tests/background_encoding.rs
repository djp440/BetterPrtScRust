//! 慢格式（AVIF）的编码不阻塞事件循环：编码在后台进行，完成后以平台事件回到循环，
//! 再做写剪贴板、自动存储、通知等后续；编码期间可以立即开始下一次捕获。
//!
//! 后台任务由假平台攥着，测试用 `run_background_jobs` 让它们跑完并取回平台会递交的事件。

use bps_core::{
    CaptureMode, CaptureRequest, Command, OutputFormat, PostCaptureAction, ResidentInstance,
};
use bps_platform::{Image, PlatformEvent, Point, Rect};
use bps_platform_fake::{FakeDisplay, FakePlatform, decode, desktop_pattern};

fn desktop() -> FakePlatform {
    FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary())
        .with_cursor_at(Point::new(10, 10))
}

fn display_capture(format: OutputFormat, actions: Vec<PostCaptureAction>) -> Command {
    let mut request = CaptureRequest::new(CaptureMode::Display, actions);
    request.output.output_format = format;
    Command::Capture(request)
}

fn avif_copy_and_save() -> Command {
    display_capture(
        OutputFormat::Avif,
        vec![
            PostCaptureAction::CopyToClipboard,
            PostCaptureAction::AutoSave,
        ],
    )
}

/// 让后台任务跑完，把平台递交的完成事件交给常驻实例。
fn finish_background(fake: &FakePlatform, instance: &mut ResidentInstance) {
    for event in fake.run_background_jobs() {
        assert!(matches!(event, PlatformEvent::BackgroundJobDone));
        instance.handle(event).unwrap();
    }
}

fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
    Image::from_rgba(width, height, rgba.repeat(width as usize * height as usize))
}

#[test]
fn avif_encoding_does_not_block_the_command_and_results_arrive_by_event() {
    let fake = desktop();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance.execute(avif_copy_and_save()).unwrap();

    assert_eq!(fake.pending_background_jobs(), 1, "AVIF 编码应交给后台");
    assert!(fake.clipboard().is_empty(), "编码完成前不写剪贴板");
    assert!(fake.files().is_empty(), "编码完成前不存盘");
    assert!(fake.notifications().is_empty(), "编码完成前不通知");

    finish_background(&fake, &mut instance);

    assert_eq!(fake.pending_background_jobs(), 0);
    assert_eq!(fake.clipboard().formats(), ["AVIF", "DIB", "Files"]);
    assert_eq!(
        fake.clipboard().dib_image().unwrap(),
        desktop_pattern(Rect::new(0, 0, 64, 48))
    );
    let files = fake.files();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(
        decode::avif_dimensions(&fake.file(&files[0]).unwrap()),
        (64, 48)
    );
    assert_eq!(fake.clipboard().files().unwrap(), files);
    let notifications = fake.notifications();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].title, "截图已保存");
}

#[test]
fn fast_formats_are_still_finished_within_the_command() {
    let fake = desktop();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(display_capture(
            OutputFormat::Png,
            vec![PostCaptureAction::CopyToClipboard],
        ))
        .unwrap();

    assert_eq!(fake.pending_background_jobs(), 0);
    assert_eq!(fake.clipboard().formats(), ["PNG", "DIB"]);
}

#[test]
fn another_capture_can_start_while_avif_is_encoding_and_results_land_in_order() {
    let fake = desktop();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    let copy = || display_capture(OutputFormat::Avif, vec![PostCaptureAction::CopyToClipboard]);

    instance.execute(copy()).unwrap();
    let second_screen = solid(64, 48, [10, 200, 30, 255]);
    fake.change_display_image(0, second_screen.clone());
    // 第一次的编码还没完成，第二次捕获照常开始，不被拒绝。
    instance.execute(copy()).unwrap();

    assert_eq!(fake.pending_background_jobs(), 2);
    assert!(fake.clipboard().is_empty());
    assert!(fake.notifications().is_empty(), "编码期间不应有失败通知");

    finish_background(&fake, &mut instance);

    assert_eq!(fake.pending_background_jobs(), 0);
    assert_eq!(
        fake.clipboard().dib_image().unwrap(),
        second_screen,
        "后提交的任务后完成，剪贴板最终是第二次的截图"
    );
}

#[test]
fn encoding_a_file_for_save_as_in_avif_runs_in_background() {
    let fake = FakePlatform::new()
        .with_display(FakeDisplay::new(Rect::new(0, 0, 400, 300)).primary())
        .with_cursor_at(Point::new(10, 10))
        .with_dir(r"D:\Shots");
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    let target = std::path::PathBuf::from(r"D:\Shots\shot.avif");
    fake.answer_save_as(Some(target.clone()));
    instance
        .execute(display_capture(
            OutputFormat::Png,
            vec![PostCaptureAction::Annotate],
        ))
        .unwrap();

    for event in fake.click_text("另存为") {
        instance.handle(event).unwrap();
    }

    assert_eq!(
        fake.pending_background_jobs(),
        1,
        "另存为 AVIF 的编码交给后台"
    );
    assert!(fake.file(&target).is_none(), "编码完成前文件还没写");

    finish_background(&fake, &mut instance);

    let bytes = fake.file(&target).expect("编码完成后写入用户选的路径");
    assert_eq!(decode::avif_dimensions(&bytes), (400, 300));
}
