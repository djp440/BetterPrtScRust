//! 自动存储：存储目录、命名模板、输出格式、同名文件处理和通知。
//!
//! 从“命令”入口驱动，通过假文件系统和假通知检查外部可见的结果；
//! 写入的文件用假平台里独立的解码器读回，与假显示器的画面比较。

use std::path::{Path, PathBuf};

use bps_core::{
    CaptureMode, CaptureRequest, Command, OutputFormat, PostCaptureAction, ResidentInstance,
    SaveDir,
};
use bps_platform::{Image, LocalDateTime, Rect};
use bps_platform_fake::{DEFAULT_PICTURES_DIR, FakeDisplay, FakePlatform, decode};

fn display() -> FakeDisplay {
    FakeDisplay::new(Rect::new(0, 0, 64, 48)).primary()
}

fn auto_save_request() -> CaptureRequest {
    CaptureRequest::new(CaptureMode::Display, vec![PostCaptureAction::AutoSave])
}

fn default_save_dir() -> PathBuf {
    Path::new(DEFAULT_PICTURES_DIR).join("BetterPrtSc")
}

#[test]
fn auto_save_writes_png_named_by_default_template_into_pictures_subfolder() {
    let fake = FakePlatform::new()
        .with_display(display())
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7));
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(Command::Capture(auto_save_request()))
        .unwrap();

    let expected_path = default_save_dir().join("2026-10-24 09-05-07.png");
    assert_eq!(fake.files(), std::slice::from_ref(&expected_path));
    let saved = decode::png(&fake.file(&expected_path).unwrap());
    assert_eq!(&saved, display().image());
}

#[test]
fn auto_save_appends_counter_instead_of_overwriting_same_name_file() {
    let existing = default_save_dir().join("2026-10-24 09-05-07.png");
    let fake = FakePlatform::new()
        .with_display(display())
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7))
        .with_file(&existing, b"existing".to_vec());
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());

    instance
        .execute(Command::Capture(auto_save_request()))
        .unwrap();
    instance
        .execute(Command::Capture(auto_save_request()))
        .unwrap();

    assert_eq!(
        fake.files(),
        [
            default_save_dir().join("2026-10-24 09-05-07-1.png"),
            default_save_dir().join("2026-10-24 09-05-07-2.png"),
            existing.clone(),
        ]
    );
    assert_eq!(fake.file(&existing).unwrap(), b"existing");
}

#[test]
fn auto_save_uses_custom_save_dir_and_filename_template() {
    let save_dir = PathBuf::from(r"D:\截图\工作");
    let fake = FakePlatform::new()
        .with_display(display())
        .with_time(LocalDateTime::new(2026, 10, 24, 9, 5, 7));
    let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
    let mut request = auto_save_request();
    request.output.save_dir = SaveDir::Custom(save_dir.clone());
    request.output.filename_template = "{mode} {HH:mm}".parse().unwrap();

    instance.execute(Command::Capture(request)).unwrap();

    assert_eq!(fake.files(), [save_dir.join("display 09_05.png")]);
}

#[test]
fn auto_save_encodes_file_in_each_output_format() {
    let time = LocalDateTime::new(2026, 10, 24, 9, 5, 7);
    for format in [
        OutputFormat::Png,
        OutputFormat::Jpeg,
        OutputFormat::WebP,
        OutputFormat::Avif,
    ] {
        let fake = FakePlatform::new().with_display(display()).with_time(time);
        let mut instance = ResidentInstance::new(fake.platform(), Vec::new());
        let mut request = auto_save_request();
        request.output.output_format = format;

        instance.execute(Command::Capture(request)).unwrap();
        // AVIF 在后台编码，完成后以事件回到常驻实例。
        for event in fake.run_background_jobs() {
            instance.handle(event).unwrap();
        }

        let expected = display().image().clone();
        let (path, bytes) = only_file(&fake);
        match format {
            OutputFormat::Png => {
                assert_eq!(file_name(&path), "2026-10-24 09-05-07.png");
                assert_eq!(decode::png(&bytes), expected);
            }
            OutputFormat::Jpeg => {
                assert_eq!(file_name(&path), "2026-10-24 09-05-07.jpg");
                assert_similar(&decode::jpeg(&bytes), &expected);
            }
            OutputFormat::WebP => {
                assert_eq!(file_name(&path), "2026-10-24 09-05-07.webp");
                assert_eq!(decode::webp(&bytes), expected, "WebP 应为无损");
            }
            OutputFormat::Avif => {
                assert_eq!(file_name(&path), "2026-10-24 09-05-07.avif");
                assert_eq!(decode::avif_dimensions(&bytes), (64, 48));
            }
        }
    }
}

fn only_file(fake: &FakePlatform) -> (PathBuf, Vec<u8>) {
    let files = fake.files();
    assert_eq!(files.len(), 1, "应只写入一个文件：{files:?}");
    let bytes = fake.file(&files[0]).unwrap();
    (files[0].clone(), bytes)
}

fn file_name(path: &Path) -> &str {
    path.file_name().unwrap().to_str().unwrap()
}

/// 有损格式：尺寸相同，且每个通道的平均误差很小。
fn assert_similar(actual: &Image, expected: &Image) {
    assert_eq!(
        (actual.width(), actual.height()),
        (expected.width(), expected.height())
    );
    let total: u64 = actual
        .rgba()
        .iter()
        .zip(expected.rgba())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = total as f64 / actual.rgba().len() as f64;
    assert!(mean < 2.0, "有损编码的平均误差过大：{mean:.2}");
}
