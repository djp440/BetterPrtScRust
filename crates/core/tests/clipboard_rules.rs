//! 剪贴板规则：图像按输出格式编码并总是附带 DIB；同时做了自动存储时附带存下的文件。

use std::path::Path;

use bps_core::{
    CaptureMode, CaptureRequest, Command, OutputFormat, PostCaptureAction, ResidentInstance,
};
use bps_platform::{ClipboardItem, Image, LocalDateTime, Rect};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, decode};

fn display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary()
}

fn fake() -> FakePlatform {
    FakePlatform::new()
        .with_display(display())
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7))
}

fn capture(actions: Vec<PostCaptureAction>, format: OutputFormat) -> Command {
    let mut request = CaptureRequest::new(CaptureMode::Display, actions);
    request.output.output_format = format;
    Command::Capture(request)
}

#[test]
fn copied_image_is_encoded_in_output_format_with_dib() {
    let fake = fake();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(capture(
            vec![PostCaptureAction::CopyToClipboard],
            OutputFormat::WebP,
        ))
        .unwrap();

    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["WebP", "DIB"]);
    let ClipboardItem::WebP(webp) = &clipboard.items()[0] else {
        unreachable!()
    };
    assert_eq!(&decode::webp(webp), display().image());
    assert_eq!(&clipboard.dib_image().unwrap(), display().image());
    assert!(fake.files().is_empty(), "只复制时不应写盘");
}

#[test]
fn copy_with_auto_save_also_puts_saved_file_on_clipboard() {
    let fake = fake();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(capture(
            vec![
                PostCaptureAction::CopyToClipboard,
                PostCaptureAction::AutoSave,
            ],
            OutputFormat::Jpeg,
        ))
        .unwrap();

    let saved = Path::new(DEFAULT_PICTURES_DIR)
        .join("BetterPrtSc")
        .join("2026-10-24 09-05-07.jpg");
    assert_eq!(fake.files(), std::slice::from_ref(&saved));
    let clipboard = fake.clipboard();
    assert_eq!(clipboard.formats(), ["JPEG", "DIB", "Files"]);
    assert_eq!(clipboard.files().unwrap(), std::slice::from_ref(&saved));
    // 剪贴板里的图像与存下的文件是同一份编码结果。
    let ClipboardItem::Jpeg(jpeg) = &clipboard.items()[0] else {
        unreachable!()
    };
    assert_eq!(jpeg, &fake.file(&saved).unwrap());
    assert_eq!(&clipboard.dib_image().unwrap(), display().image());
}

#[test]
fn auto_save_without_copy_leaves_clipboard_untouched() {
    let fake = fake();
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(capture(
            vec![PostCaptureAction::AutoSave],
            OutputFormat::Png,
        ))
        .unwrap();

    assert!(fake.clipboard().is_empty());
    assert_eq!(fake.files().len(), 1);
}

#[test]
fn jpeg_output_fills_transparent_pixels_with_white() {
    let transparent = Image::from_rgba(64, 48, [0, 0, 0, 0].repeat(64 * 48));
    let fake = FakePlatform::new().with_display(display().with_image(transparent.clone()));
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(capture(
            vec![
                PostCaptureAction::CopyToClipboard,
                PostCaptureAction::AutoSave,
            ],
            OutputFormat::Jpeg,
        ))
        .unwrap();

    let saved = decode::jpeg(&fake.file(&fake.files()[0]).unwrap());
    assert!(
        saved
            .rgba()
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|&c| c >= 250)),
        "JPEG 中透明部分应为白色"
    );
    // DIB 保留原始像素（含 alpha），不受输出格式影响。
    assert_eq!(fake.clipboard().dib_image().unwrap(), transparent);
}
