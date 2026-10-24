use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bps_platform::{Clipboard, ClipboardItem, Image, PlatformError};

use crate::decode;

/// 剪贴板某一时刻的内容，提供把各格式解码回图像的辅助方法（见 [`decode`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardContent {
    items: Vec<ClipboardItem>,
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[ClipboardItem] {
        &self.items
    }

    /// 剪贴板中的格式名称，按写入顺序排列，例如 `["PNG", "DIB"]`。
    pub fn formats(&self) -> Vec<&'static str> {
        self.items.iter().map(ClipboardItem::format_name).collect()
    }

    /// 把 PNG 格式解码成图像；没有 PNG 格式时返回 `None`，解码失败时 panic。
    pub fn png_image(&self) -> Option<Image> {
        self.items.iter().find_map(|item| match item {
            ClipboardItem::Png(bytes) => Some(decode::png(bytes)),
            _ => None,
        })
    }

    /// 把 DIB 格式解码成图像；没有 DIB 格式时返回 `None`，格式不合法时 panic。
    pub fn dib_image(&self) -> Option<Image> {
        self.items.iter().find_map(|item| match item {
            ClipboardItem::Dib(bytes) => Some(decode::dib(bytes)),
            _ => None,
        })
    }

    /// 剪贴板里附带的文件；没有文件格式时返回 `None`。
    pub fn files(&self) -> Option<Vec<PathBuf>> {
        self.items.iter().find_map(|item| match item {
            ClipboardItem::Files(paths) => Some(paths.clone()),
            _ => None,
        })
    }
}

pub(crate) struct FakeClipboard(pub Arc<Mutex<Option<ClipboardContent>>>);

impl Clipboard for FakeClipboard {
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError> {
        *self.0.lock().unwrap() = Some(ClipboardContent { items });
        Ok(())
    }
}
