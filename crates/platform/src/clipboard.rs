use std::path::PathBuf;

use crate::PlatformError;

/// 写入剪贴板的一种数据格式；一次写入可以同时放入多种格式。
///
/// 图像字节由 `core` 编码好，平台实现只负责按对应的系统格式放进剪贴板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardItem {
    /// PNG 文件的完整字节。
    Png(Vec<u8>),
    /// JPEG 文件的完整字节。
    Jpeg(Vec<u8>),
    /// WebP 文件的完整字节。
    WebP(Vec<u8>),
    /// AVIF 文件的完整字节。
    Avif(Vec<u8>),
    /// 设备无关位图：`BITMAPINFOHEADER`（40 字节，32 位，`BI_RGB`）后接
    /// 自下而上的 BGRA 像素，即 Windows `CF_DIB` 的内容。
    Dib(Vec<u8>),
    /// 磁盘上的文件（绝对路径），在文件管理器里可以直接粘贴成文件。
    Files(Vec<PathBuf>),
}

impl ClipboardItem {
    /// 格式的简短名称，便于日志和测试断言。
    pub fn format_name(&self) -> &'static str {
        match self {
            ClipboardItem::Png(_) => "PNG",
            ClipboardItem::Jpeg(_) => "JPEG",
            ClipboardItem::WebP(_) => "WebP",
            ClipboardItem::Avif(_) => "AVIF",
            ClipboardItem::Dib(_) => "DIB",
            ClipboardItem::Files(_) => "Files",
        }
    }
}

/// 系统剪贴板。
pub trait Clipboard {
    /// 清空剪贴板，然后一次性写入 `items` 中的所有格式。
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError>;
}
