//! 真机冒烟测试：用系统的 `Windows.Media.Ocr` 识别在内存里画出的中英文文字，默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows --test ocr_smoke -- --ignored --nocapture`
//! 需要系统装有中文（简体）和英文的 OCR 语言包。只在内存位图上画字，不显示窗口、不碰剪贴板，
//! 也不发出网络请求。
#![cfg(windows)]

use bps_platform::{Image, TextRecognizer};
use bps_platform_windows::WindowsTextRecognizer;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_NORMAL, FillRect, GdiFlush, GetStockObject,
    OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW, WHITE_BRUSH,
};
use windows::core::w;

/// 用微软雅黑在白底上画出 `lines`（字高 `size` 像素），返回 RGBA 图像。
fn render_text(lines: &[&str], size: i32) -> Image {
    let line_height = size * 2;
    let (width, height) = (size * 24, line_height * lines.len() as i32 + size);
    unsafe {
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .expect("创建位图");
        let old_bitmap = SelectObject(dc, bitmap.into());
        let all = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        FillRect(
            dc,
            &all,
            windows::Win32::Graphics::Gdi::HBRUSH(GetStockObject(WHITE_BRUSH).0),
        );
        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        );
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0));
        for (i, line) in lines.iter().enumerate() {
            let text: Vec<u16> = line.encode_utf16().collect();
            let _ = TextOutW(dc, size, size / 2 + line_height * i as i32, &text);
        }
        let _ = GdiFlush();
        let bgra = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
        let rgba = bgra
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        Image::from_rgba(width as u32, height as u32, rgba)
    }
}

/// 所有词连在一起（去掉词之间的分隔），便于不受分词方式影响地检查内容。
fn squashed(lines: &[Vec<String>]) -> String {
    lines.iter().flatten().map(String::as_str).collect()
}

#[test]
#[ignore = "真机冒烟：需要中文（简体）和英文的 OCR 语言包"]
fn recognizes_mixed_chinese_and_english_text() {
    let recognizer = WindowsTextRecognizer;
    let languages = recognizer.languages().unwrap();
    println!("OCR 语言包：{languages:?}");
    println!("默认识别语言：{:?}", recognizer.default_language());
    let chinese = languages
        .iter()
        .find(|l| l.tag.starts_with("zh-Hans"))
        .expect("需要中文（简体）的 OCR 语言包");

    let image = render_text(
        &[
            "BetterPrtSc 截图工具",
            "Hello World 你好世界",
            "使用Windows识别文字，版本 2.0",
        ],
        32,
    );
    let started = std::time::Instant::now();
    let lines = recognizer.recognize(&image, &chinese.tag).unwrap();
    println!("识别耗时 {:?}：{lines:?}", started.elapsed());

    // Windows 的中文 OCR 对中英混排的识别并不完美（常把 l 认成 I、个别汉字拆成偏旁），
    // 这里只检查中英文都被识别出来、且按行返回。
    assert_eq!(lines.len(), 3, "{lines:?}");
    let text = squashed(&lines);
    for expected in ["BetterPrtSc", "截图", "世界", "版本"] {
        assert!(text.contains(expected), "缺少“{expected}”：{lines:?}");
    }
}

#[test]
#[ignore = "真机冒烟：需要英文的 OCR 语言包"]
fn recognizes_english_text_with_the_english_language_pack() {
    let recognizer = WindowsTextRecognizer;
    let image = render_text(&["The quick brown fox"], 24);

    let lines = recognizer.recognize(&image, "en-US").unwrap();
    println!("{lines:?}");

    assert_eq!(lines, [["The", "quick", "brown", "fox"]]);
}

#[test]
#[ignore = "真机冒烟：OCR 语言包"]
fn default_language_is_one_of_the_installed_languages() {
    let recognizer = WindowsTextRecognizer;
    let languages = recognizer.languages().unwrap();
    let default = recognizer.default_language().expect("本机应有默认识别语言");

    assert!(languages.iter().any(|l| l.tag == default), "{languages:?}");
    assert!(languages.iter().all(|l| !l.name.is_empty()));
}

#[test]
#[ignore = "真机冒烟：OCR 对极小或空白图像的表现"]
fn blank_and_tiny_images_give_no_text() {
    let recognizer = WindowsTextRecognizer;
    let tag = recognizer.default_language().unwrap();
    for (width, height) in [(1, 1), (40, 12), (300, 100)] {
        let blank = Image::from_rgba(width, height, vec![255; (width * height * 4) as usize]);
        let result = recognizer.recognize(&blank, &tag);
        println!("{width}×{height}：{result:?}");
        assert_eq!(result.unwrap(), Vec::<Vec<String>>::new());
    }
}
