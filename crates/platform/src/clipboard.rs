use crate::PlatformError;

/// 写入剪贴板的一种数据格式；一次写入可以同时放入多种格式。
///
/// 字节内容由 `core` 编码好，平台实现只负责按对应的系统格式放进剪贴板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardItem {
    /// PNG 文件的完整字节。
    Png(Vec<u8>),
    /// 设备无关位图：`BITMAPINFOHEADER`（40 字节，32 位，`BI_RGB`）后接
    /// 自下而上的 BGRA 像素，即 Windows `CF_DIB` 的内容。
    Dib(Vec<u8>),
}

impl ClipboardItem {
    /// 格式的简短名称，便于日志和测试断言。
    pub fn format_name(&self) -> &'static str {
        match self {
            ClipboardItem::Png(_) => "PNG",
            ClipboardItem::Dib(_) => "DIB",
        }
    }
}

/// 系统剪贴板。
pub trait Clipboard {
    /// 清空剪贴板，然后一次性写入 `items` 中的所有格式。
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError>;
}
