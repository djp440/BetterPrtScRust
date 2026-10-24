use crate::{Image, PlatformError};

/// 识别语言：系统已为它安装了 OCR 语言包，可以用来识别文字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecognitionLanguage {
    /// BCP 47 语言标签，如 `zh-Hans-CN`、`en-US`。
    pub tag: String,
    /// 给用户看的名称，用这种语言自己的写法，如 `English (United States)`。
    pub name: String,
}

/// 识别出的一行文字：按阅读顺序排列的词。
///
/// 词之间原本是否有空格由 `core` 判断（中文等语言的词之间没有空格），平台实现只给出各个词。
pub type RecognizedLine = Vec<String>;

/// 文字识别（OCR）：在本机识别图像里的文字，不发出任何网络请求（ADR 0004）。
///
/// 只在结果面板的子进程里使用，常驻实例不加载它。
pub trait TextRecognizer {
    /// 系统已安装 OCR 语言包的全部识别语言；一个都没有时为空。
    fn languages(&self) -> Result<Vec<RecognitionLanguage>, PlatformError>;

    /// 跟随系统的默认识别语言：用户的语言首选项里第一个已安装 OCR 语言包的语言（标签）；
    /// 首选项里的语言都没有安装时为 `None`。
    fn default_language(&self) -> Option<String>;

    /// 用识别语言 `language`（[`languages`](Self::languages) 里的标签）识别 `image` 中的文字，
    /// 按阅读顺序返回各行。可能耗时几百毫秒，界面应在后台线程调用。
    fn recognize(
        &self,
        image: &Image,
        language: &str,
    ) -> Result<Vec<RecognizedLine>, PlatformError>;
}
