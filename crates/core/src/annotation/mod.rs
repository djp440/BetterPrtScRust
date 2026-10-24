//! 标注：标注文档模型（标注列表 → 合成后的截图）与标注时的交互。
//!
//! 合成完全在 `core` 里用软件光栅化完成（[`AnnotationDocument::compose`]），选区遮罩上显示的
//! 也是同一个函数合成的像素（遮罩的画布），所以屏幕上看到的就是最终产物。
//!
//! 坐标是截图的像素坐标（截图左上角为原点），与截图在屏幕上的位置无关：区域捕获时截图
//! 就是区域，长截屏等其他来源的截图同样适用。
//!
//! 扩展点（后续 ticket）：新的标注种类在 [`Annotation`] 加变体，并在 `paint` / `bounds` /
//! `footprint` 里各加一个分支（马赛克从已合成的像素取色、文字的字形由平台提供后存进标注）；
//! 撤销 / 重做与再编辑经 [`AnnotationDocument`] 上的修改方法实现。

pub(crate) mod finish;
mod raster;
pub(crate) mod session;
pub(crate) mod toolbar;

use bps_platform::{Color, Image, Point, Rect};

use raster::{Polygon, Target, rect_from_edges};

/// 标注的线条样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stroke {
    pub color: Color,
    /// 线宽（截图的物理像素）；0 按 1 处理。
    pub width: u32,
}

/// 叠加在截图上的一个标注。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Annotation {
    /// 框：矩形的轮廓，线条画在 `rect` 以内（外缘与 `rect` 重合）。
    Rectangle { rect: Rect, stroke: Stroke },
    /// 箭头：从 `from` 像素的中心指向 `to` 像素的中心，箭头尖端在 `to`。箭杆宽 `stroke.width`，
    /// 箭头的大小随线宽变化；箭头比整支箭还长时按比例缩小。
    Arrow {
        from: Point,
        to: Point,
        stroke: Stroke,
    },
}

impl Annotation {
    /// 这个标注可能画到的像素范围（含抗锯齿的边缘），可能超出截图。
    pub fn bounds(&self) -> Rect {
        match *self {
            Annotation::Rectangle { rect, .. } => rect,
            Annotation::Arrow { from, to, stroke } => arrow_polygons(from, to, stroke.width)
                .iter()
                .flatten()
                .map(|&(x, y)| rect_from_edges(x - 2.0, y - 2.0, x + 2.0, y + 2.0))
                .reduce(|a, b| a.union(b))
                .unwrap_or_default(),
        }
    }

    /// 实际被画到的像素所在的几个矩形（并集覆盖全部被画到的像素），用于局部刷新：
    /// 框只有四条边，不必刷新框内部。
    pub(crate) fn footprint(&self) -> Vec<Rect> {
        match *self {
            Annotation::Rectangle { rect, stroke } => rectangle_strips(rect, stroke.width),
            Annotation::Arrow { .. } => vec![self.bounds()],
        }
    }

    /// 按比例缩放后的标注（横向 `scale_x`、纵向 `scale_y` 倍，线宽按两者的平均值缩放，
    /// 至少 1 像素）：截图缩小显示时，屏幕上画的就是它。
    pub(crate) fn scaled(&self, scale_x: f64, scale_y: f64) -> Annotation {
        let point = |p: Point| {
            Point::new(
                (f64::from(p.x) * scale_x).round() as i32,
                (f64::from(p.y) * scale_y).round() as i32,
            )
        };
        let stroke = |s: Stroke| Stroke {
            width: ((f64::from(s.width.max(1)) * (scale_x + scale_y) / 2.0).round() as u32).max(1),
            ..s
        };
        match *self {
            Annotation::Rectangle { rect, stroke: s } => {
                let from = point(Point::new(rect.x, rect.y));
                let to = point(Point::new(
                    rect.x + rect.width as i32,
                    rect.y + rect.height as i32,
                ));
                Annotation::Rectangle {
                    rect: Rect::new(
                        from.x,
                        from.y,
                        (to.x - from.x).max(1) as u32,
                        (to.y - from.y).max(1) as u32,
                    ),
                    stroke: stroke(s),
                }
            }
            Annotation::Arrow {
                from,
                to,
                stroke: s,
            } => Annotation::Arrow {
                from: point(from),
                to: point(to),
                stroke: stroke(s),
            },
        }
    }

    fn paint(&self, target: &mut Target<'_>) {
        match *self {
            Annotation::Rectangle { rect, stroke } => {
                for strip in rectangle_strips(rect, stroke.width) {
                    target.fill_rect(strip, stroke.color);
                }
            }
            Annotation::Arrow { from, to, stroke } => {
                target.fill_polygons(&arrow_polygons(from, to, stroke.width), stroke.color);
            }
        }
    }
}

/// 标注文档：叠加在一张截图上的标注，按添加顺序从下往上画。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotationDocument {
    annotations: Vec<Annotation>,
}

impl AnnotationDocument {
    /// 没有任何标注的文档。
    pub fn new() -> Self {
        Self::default()
    }

    /// 在最上面加一个标注。
    pub fn add(&mut self, annotation: Annotation) {
        self.annotations.push(annotation);
    }

    /// 全部标注，从下往上排列。
    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    /// 把全部标注画到截图 `base` 上，得到最终的截图（尺寸与 `base` 相同）。
    /// 超出截图的部分被裁掉。
    pub fn compose(&self, base: &Image) -> Image {
        compose_area(
            &self.annotations,
            base,
            Rect::new(0, 0, base.width(), base.height()),
        )
    }
}

/// 只合成截图上 `area` 这一块（截图像素坐标，超出截图的部分被裁掉）：把 `annotations`
/// 依次画到 `base` 对应的像素上。与 [`AnnotationDocument::compose`] 结果中同一块像素完全相同。
pub(crate) fn compose_area(annotations: &[Annotation], base: &Image, area: Rect) -> Image {
    let Some(area) = area.intersection(Rect::new(0, 0, base.width(), base.height())) else {
        return Image::from_rgba(0, 0, Vec::new());
    };
    let (base_width, left) = (base.width() as usize, area.x as usize);
    let row_bytes = area.width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * area.height as usize);
    for y in area.y as usize..area.y as usize + area.height as usize {
        let start = (y * base_width + left) * 4;
        pixels.extend_from_slice(&base.rgba()[start..start + row_bytes]);
    }
    let mut target = Target {
        pixels: &mut pixels,
        area,
    };
    for annotation in annotations {
        if annotation.bounds().intersection(area).is_some() {
            annotation.paint(&mut target);
        }
    }
    Image::from_rgba(area.width, area.height, pixels)
}

/// 框的四条边：上、下两条横贯整个宽度，左、右两条夹在中间。框太小时就是整个矩形。
fn rectangle_strips(rect: Rect, width: u32) -> Vec<Rect> {
    let width = width.max(1);
    if rect.width <= 2 * width || rect.height <= 2 * width {
        return vec![rect];
    }
    let (right, bottom) = (
        rect.x + (rect.width - width) as i32,
        rect.y + (rect.height - width) as i32,
    );
    let middle = rect.height - 2 * width;
    vec![
        Rect::new(rect.x, rect.y, rect.width, width),
        Rect::new(rect.x, bottom, rect.width, width),
        Rect::new(rect.x, rect.y + width as i32, width, middle),
        Rect::new(right, rect.y + width as i32, width, middle),
    ]
}

/// 箭头的形状：箭杆（矩形）和箭头（三角形）两个凸多边形。两端重合时为空。
fn arrow_polygons(from: Point, to: Point, width: u32) -> Vec<Polygon> {
    let start = (from.x as f32 + 0.5, from.y as f32 + 0.5);
    let tip = (to.x as f32 + 0.5, to.y as f32 + 0.5);
    let (dx, dy) = (tip.0 - start.0, tip.1 - start.1);
    let length = dx.hypot(dy);
    if length < 0.5 {
        return Vec::new();
    }
    let (ux, uy) = (dx / length, dy / length);
    let (nx, ny) = (-uy, ux);
    let width = width.max(1) as f32;
    let head_length = (4.0 * width + 6.0).min(length);
    let head_half_width = 0.6 * head_length;
    let at = |along: f32, across: f32| {
        (
            tip.0 - ux * along + nx * across,
            tip.1 - uy * along + ny * across,
        )
    };
    let mut polygons = vec![vec![
        tip,
        at(head_length, head_half_width),
        at(head_length, -head_half_width),
    ]];
    // 箭杆伸进箭头一半，两块之间不留缝；那里的箭头已经比箭杆宽。
    let shaft_end = head_length * 0.5;
    if length > shaft_end {
        let half = width / 2.0;
        polygons.push(vec![
            at(length, half),
            at(shaft_end, half),
            at(shaft_end, -half),
            at(length, -half),
        ]);
    }
    polygons
}
