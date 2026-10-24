//! 文字识别：系统自带的 `Windows.Media.Ocr`，完全在本机识别，不发出网络请求（ADR 0004）。
//!
//! 识别语言取决于系统安装了哪些 OCR 语言包（随语言一起在“设置 > 时间和语言 > 语言和区域”
//! 里安装）。只在结果面板子进程里使用，常驻实例不加载它。

use std::borrow::Cow;

use bps_platform::{Image, PlatformError, RecognitionLanguage, RecognizedLine, TextRecognizer};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::core::HSTRING;

use crate::error::win_error;

/// `Windows.Media.Ocr` 的文字识别。不持有任何状态，可以在任意线程使用。
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsTextRecognizer;

impl TextRecognizer for WindowsTextRecognizer {
    fn languages(&self) -> Result<Vec<RecognitionLanguage>, PlatformError> {
        let fail = |e| win_error("查询 OCR 语言包失败", e);
        OcrEngine::AvailableRecognizerLanguages()
            .map_err(fail)?
            .into_iter()
            .map(|language| {
                Ok(RecognitionLanguage {
                    tag: language.LanguageTag().map_err(fail)?.to_string(),
                    name: language.NativeName().map_err(fail)?.to_string(),
                })
            })
            .collect()
    }

    fn default_language(&self) -> Option<String> {
        // 用户的语言首选项里没有一种装了 OCR 语言包时，系统返回空引擎（这里是错误）。
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().ok()?;
        let tag = engine.RecognizerLanguage().ok()?.LanguageTag().ok()?;
        Some(tag.to_string())
    }

    fn recognize(
        &self,
        image: &Image,
        language: &str,
    ) -> Result<Vec<RecognizedLine>, PlatformError> {
        let fail = |e| win_error("识别文字失败", e);
        let language = Language::CreateLanguage(&HSTRING::from(language)).map_err(fail)?;
        let engine = OcrEngine::TryCreateFromLanguage(&language)
            .map_err(|e| win_error("这种语言的 OCR 语言包不可用", e))?;
        let max = OcrEngine::MaxImageDimension().map_err(fail)?;
        let bitmap = software_bitmap(&fit_within(image, max)).map_err(fail)?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(fail)?
            .join()
            .map_err(fail)?;
        result
            .Lines()
            .map_err(fail)?
            .into_iter()
            .map(|line| {
                line.Words()
                    .map_err(fail)?
                    .into_iter()
                    .map(|word| Ok(word.Text().map_err(fail)?.to_string()))
                    .collect()
            })
            .collect()
    }
}

/// 把截图转换成 OCR 引擎要的 BGRA 位图；半透明处（如虚拟桌面上显示器之间的空隙）合成到白底上。
fn software_bitmap(image: &Image) -> windows::core::Result<SoftwareBitmap> {
    let mut bgra = Vec::with_capacity(image.rgba().len());
    for pixel in image.rgba().chunks_exact(4) {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        let over_white =
            |c: u8| ((u32::from(c) * u32::from(a) + 255 * u32::from(255 - a)) / 255) as u8;
        bgra.extend_from_slice(&[over_white(b), over_white(g), over_white(r), 255]);
    }
    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra)?;
    SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
    )
}

/// 宽或高超过 OCR 引擎的上限时按比例缩小（最近邻取样，足够识别用）。
fn fit_within(image: &Image, max: u32) -> Cow<'_, Image> {
    let (width, height) = (image.width(), image.height());
    if width <= max && height <= max {
        return Cow::Borrowed(image);
    }
    let scale = f64::from(max) / f64::from(width.max(height));
    let new_width = ((f64::from(width) * scale) as u32).clamp(1, max);
    let new_height = ((f64::from(height) * scale) as u32).clamp(1, max);
    let mut rgba = Vec::with_capacity(new_width as usize * new_height as usize * 4);
    for y in 0..new_height {
        let source_y = (u64::from(y) * u64::from(height) / u64::from(new_height)) as u32;
        for x in 0..new_width {
            let source_x = (u64::from(x) * u64::from(width) / u64::from(new_width)) as u32;
            rgba.extend_from_slice(&image.pixel(source_x, source_y));
        }
    }
    Cow::Owned(Image::from_rgba(new_width, new_height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_within_the_limit_are_used_as_they_are() {
        let image = Image::from_rgba(3, 2, vec![9; 24]);
        assert!(matches!(fit_within(&image, 3), Cow::Borrowed(_)));
    }

    #[test]
    fn images_over_the_limit_are_scaled_down_keeping_the_aspect_ratio() {
        let image = Image::from_rgba(40, 10, vec![200; 40 * 10 * 4]);
        let fitted = fit_within(&image, 20);
        assert_eq!((fitted.width(), fitted.height()), (20, 5));
        assert_eq!(fitted.pixel(19, 4), [200; 4]);
    }
}
