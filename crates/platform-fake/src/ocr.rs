use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bps_platform::{Image, PlatformError, RecognitionLanguage, RecognizedLine, TextRecognizer};

#[derive(Default)]
pub(crate) struct OcrState {
    /// 已安装 OCR 语言包的识别语言。
    pub languages: Vec<RecognitionLanguage>,
    /// 跟随系统的默认识别语言。
    pub default_language: Option<String>,
    /// 用各识别语言识别时得到的文字；没有设置的语言识别不出任何文字。
    pub results: HashMap<String, Vec<RecognizedLine>>,
    /// 识别会以这条说明失败。
    pub failure: Option<String>,
    /// 至今的每次识别：被识别的图像和所用的识别语言。
    pub requests: Vec<(Image, String)>,
}

/// 假的文字识别：返回测试预先设置的文字，并记下每次识别的图像和识别语言。
pub(crate) struct FakeTextRecognizer(pub Arc<Mutex<OcrState>>);

impl TextRecognizer for FakeTextRecognizer {
    fn languages(&self) -> Result<Vec<RecognitionLanguage>, PlatformError> {
        Ok(self.0.lock().unwrap().languages.clone())
    }

    fn default_language(&self) -> Option<String> {
        self.0.lock().unwrap().default_language.clone()
    }

    fn recognize(
        &self,
        image: &Image,
        language: &str,
    ) -> Result<Vec<RecognizedLine>, PlatformError> {
        let mut state = self.0.lock().unwrap();
        assert!(
            state.languages.iter().any(|l| l.tag == language),
            "识别语言 {language} 没有安装 OCR 语言包"
        );
        state.requests.push((image.clone(), language.to_owned()));
        if let Some(message) = &state.failure {
            return Err(PlatformError::new(message.clone()));
        }
        Ok(state.results.get(language).cloned().unwrap_or_default())
    }
}
