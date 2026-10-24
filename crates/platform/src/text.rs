use std::fmt;

use crate::Rect;

/// 排版并栅格化好的一段文字：灰度抗锯齿的覆盖率蒙版，由平台的字体引擎生成
/// （[`SelectionOverlay::rasterize_text`](crate::SelectionOverlay::rasterize_text)），
/// `core` 按颜色把它叠到截图上。
///
/// 坐标是“排版坐标”：物理像素，原点是第一行文字排版框的左上角。
#[derive(Clone, PartialEq, Eq)]
pub struct TextMask {
    area: Rect,
    coverage: Vec<u8>,
    layout: Rect,
    caret: Rect,
}

impl TextMask {
    /// `coverage` 的长度必须等于 `area.width × area.height`。
    pub fn new(area: Rect, coverage: Vec<u8>, layout: Rect, caret: Rect) -> Self {
        assert_eq!(
            coverage.len(),
            area.width as usize * area.height as usize,
            "覆盖率蒙版的长度与 {}x{} 不符",
            area.width,
            area.height
        );
        Self {
            area,
            coverage,
            layout,
            caret,
        }
    }

    /// 蒙版覆盖的范围。字形可能伸出排版框，所以左上角可以为负；没有可见字形时宽或高为 0。
    pub fn area(&self) -> Rect {
        self.area
    }

    /// 每个像素的覆盖率（0 = 不画，255 = 完全盖住），按行从上到下，行间无填充。
    pub fn coverage(&self) -> &[u8] {
        &self.coverage
    }

    /// 排版框：全部文字行（含行尾空格）占据的范围，左上角是原点；没有文字时宽为 0、
    /// 高为一行。
    pub fn layout(&self) -> Rect {
        self.layout
    }

    /// 文字末尾的插入点：一条竖线所占的矩形。
    pub fn caret(&self) -> Rect {
        self.caret
    }
}

impl fmt::Debug for TextMask {
    // 不打印覆盖率。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextMask")
            .field("area", &self.area)
            .field("layout", &self.layout)
            .field("caret", &self.caret)
            .finish_non_exhaustive()
    }
}
