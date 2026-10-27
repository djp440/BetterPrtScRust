//! 标注的软件光栅化：在截图的一块像素上画实心矩形、带抗锯齿的凸多边形、覆盖率蒙版（文字）
//! 和马赛克。
//!
//! 坐标是截图的像素坐标；像素 `(x, y)` 占据 `[x, x+1) × [y, y+1)`，中心在 `(x+0.5, y+0.5)`。
//! 像素是非预乘 alpha 的 RGBA8，颜色按 src-over 叠加。

use bps_platform::{Color, Rect};

/// 截图上正在合成的一块：`area`（截图像素坐标）范围内的 RGBA 像素，行间无填充。
pub(super) struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub area: Rect,
}

/// 凸多边形，顶点按顺序排列（顺时针、逆时针都可以）。
pub(super) type Polygon = Vec<(f32, f32)>;

impl Target<'_> {
    /// 用 `color` 填满 `rect` 与本块的重叠部分。
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let Some(rect) = rect.intersection(self.area) else {
            return;
        };
        for y in rect.y..rect.y + rect.height as i32 {
            let start = self.offset(rect.x, y);
            let row = &mut self.pixels[start..start + rect.width as usize * 4];
            for pixel in row.chunks_exact_mut(4) {
                blend(pixel, color, 1.0);
            }
        }
    }

    /// 用 `color` 画若干凸多边形的并集，边缘按到边的距离做抗锯齿。
    ///
    /// 每个像素的覆盖率取它在各多边形中覆盖率的最大值，重叠处不会叠深。
    pub fn fill_polygons(&mut self, polygons: &[Polygon], color: Color) {
        let shapes: Vec<Shape> = polygons.iter().filter_map(|p| Shape::new(p)).collect();
        let Some(bounds) = shapes
            .iter()
            .map(|s| s.bounds)
            .reduce(|a, b| a.union(b))
            .and_then(|b| b.intersection(self.area))
        else {
            return;
        };
        for y in bounds.y..bounds.y + bounds.height as i32 {
            let center_y = y as f32 + 0.5;
            let Some((left, right)) = shapes
                .iter()
                .filter_map(|s| s.span(center_y))
                .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            else {
                continue;
            };
            let left = left.max(bounds.x);
            let right = right.min(bounds.x + bounds.width as i32 - 1);
            for x in left..=right {
                let center = (x as f32 + 0.5, center_y);
                let coverage = shapes
                    .iter()
                    .map(|s| s.coverage(center))
                    .fold(0.0f32, f32::max);
                if coverage > 0.0 {
                    let start = self.offset(x, y);
                    blend(&mut self.pixels[start..start + 4], color, coverage);
                }
            }
        }
    }

    /// 用 `color` 按覆盖率蒙版画：`coverage` 是 `rect` 范围内每个像素的覆盖率（0–255，
    /// 按行排列），只画与本块重叠的部分。
    pub fn fill_mask(&mut self, rect: Rect, coverage: &[u8], color: Color) {
        let Some(visible) = rect.intersection(self.area) else {
            return;
        };
        for y in visible.y..visible.y + visible.height as i32 {
            let row = (y - rect.y) as usize * rect.width as usize;
            let mask_start = row + (visible.x - rect.x) as usize;
            let mask = &coverage[mask_start..mask_start + visible.width as usize];
            let start = self.offset(visible.x, y);
            let pixels = &mut self.pixels[start..start + visible.width as usize * 4];
            for (pixel, &value) in pixels.chunks_exact_mut(4).zip(mask) {
                if value > 0 {
                    blend(pixel, color, f32::from(value) / 255.0);
                }
            }
        }
    }

    /// 马赛克：`rect` 与本块重叠的部分换成边长 `block` 的色块（从 `rect` 左上角起排列），
    /// 每块的颜色是本块里这个色块整块范围内像素的平均色（按 alpha 加权）。
    pub fn pixelate(&mut self, rect: Rect, block: u32) {
        let Some(visible) = rect.intersection(self.area) else {
            return;
        };
        let step = block as i64;
        let cells = |from: i32, start: i32, length: u32| {
            let first = (i64::from(from) - i64::from(start)) / step;
            let last = (i64::from(from) + i64::from(length) - 1 - i64::from(start)) / step;
            first..=last
        };
        for row in cells(visible.y, rect.y, visible.height) {
            for column in cells(visible.x, rect.x, visible.width) {
                let cell = Rect::new(
                    (i64::from(rect.x) + column * step) as i32,
                    (i64::from(rect.y) + row * step) as i32,
                    block,
                    block,
                );
                let Some(sample) = cell.intersection(self.area) else {
                    continue;
                };
                let color = self.average(sample);
                let Some(fill) = cell.intersection(visible) else {
                    continue;
                };
                for y in fill.y..fill.y + fill.height as i32 {
                    let start = self.offset(fill.x, y);
                    for pixel in
                        self.pixels[start..start + fill.width as usize * 4].chunks_exact_mut(4)
                    {
                        pixel.copy_from_slice(&color);
                    }
                }
            }
        }
    }

    /// 本块中 `rect`（在本块以内）的平均颜色：颜色按 alpha 加权，alpha 取平均。
    fn average(&self, rect: Rect) -> [u8; 4] {
        let (mut sums, mut alpha) = ([0u64; 3], 0u64);
        for y in rect.y..rect.y + rect.height as i32 {
            let start = self.offset(rect.x, y);
            for pixel in self.pixels[start..start + rect.width as usize * 4].chunks_exact(4) {
                let a = u64::from(pixel[3]);
                for (sum, &channel) in sums.iter_mut().zip(&pixel[..3]) {
                    *sum += u64::from(channel) * a;
                }
                alpha += a;
            }
        }
        if alpha == 0 {
            return [0, 0, 0, 0];
        }
        let count = u64::from(rect.width) * u64::from(rect.height);
        let channel = |sum: u64| ((sum + alpha / 2) / alpha) as u8;
        [
            channel(sums[0]),
            channel(sums[1]),
            channel(sums[2]),
            ((alpha + count / 2) / count) as u8,
        ]
    }

    fn offset(&self, x: i32, y: i32) -> usize {
        let column = (x - self.area.x) as usize;
        let row = (y - self.area.y) as usize;
        (row * self.area.width as usize + column) * 4
    }
}

/// 预先算好边的凸多边形。
struct Shape {
    vertices: Vec<(f32, f32)>,
    /// 每条边：起点和指向多边形内部的单位法向量。
    edges: Vec<((f32, f32), (f32, f32))>,
    /// 可能被画到的像素范围（含抗锯齿的边缘）。
    bounds: Rect,
}

impl Shape {
    fn new(vertices: &[(f32, f32)]) -> Option<Self> {
        if vertices.len() < 3 {
            return None;
        }
        let pairs = || {
            vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .map(|(&a, &b)| (a, b))
        };
        let twice_area: f32 = pairs().map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum();
        if twice_area.abs() < 1e-6 {
            return None;
        }
        // y 轴向下：面积为正时顶点是顺时针，内侧在每条边的右手边。
        let side = twice_area.signum();
        let edges = pairs()
            .filter_map(|(a, b)| {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let length = dx.hypot(dy);
                (length > 1e-6).then(|| (a, (-dy * side / length, dx * side / length)))
            })
            .collect();
        let (mut left, mut top) = (f32::MAX, f32::MAX);
        let (mut right, mut bottom) = (f32::MIN, f32::MIN);
        for &(x, y) in vertices {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        // 抗锯齿让锐角顶点外侧也有一点覆盖，多留 2 个像素。
        let bounds = rect_from_edges(left - 2.0, top - 2.0, right + 2.0, bottom + 2.0);
        Some(Self {
            vertices: vertices.to_vec(),
            edges,
            bounds,
        })
    }

    /// 像素中心在 `point` 的像素被覆盖的比例：到各边的最小有符号距离 + 0.5，限制在 0..=1。
    fn coverage(&self, point: (f32, f32)) -> f32 {
        let distance = self
            .edges
            .iter()
            .map(|&((ax, ay), (nx, ny))| (point.0 - ax) * nx + (point.1 - ay) * ny)
            .fold(f32::MAX, f32::min);
        (distance + 0.5).clamp(0.0, 1.0)
    }

    /// 像素中心在 `center_y` 这一行里可能被覆盖的像素列范围（含两端）。
    fn span(&self, center_y: f32) -> Option<(i32, i32)> {
        let (top, bottom) = (center_y - 1.5, center_y + 1.5);
        let mut left = f32::MAX;
        let mut right = f32::MIN;
        let count = self.vertices.len();
        for i in 0..count {
            let (a, b) = (self.vertices[i], self.vertices[(i + 1) % count]);
            let (low, high) = (a.1.min(b.1), a.1.max(b.1));
            if high < top || low > bottom {
                continue;
            }
            let x_at = |y: f32| {
                if (b.1 - a.1).abs() < 1e-6 {
                    (a.0, b.0)
                } else {
                    let t = ((y - a.1) / (b.1 - a.1)).clamp(0.0, 1.0);
                    let x = a.0 + (b.0 - a.0) * t;
                    (x, x)
                }
            };
            for (x0, x1) in [x_at(top.max(low)), x_at(bottom.min(high))] {
                left = left.min(x0.min(x1));
                right = right.max(x0.max(x1));
            }
        }
        (left <= right).then(|| ((left - 2.0).floor() as i32, (right + 2.0).ceil() as i32))
    }
}

/// 以四条边（可以是小数）构造包含它们的整数像素矩形。
pub(super) fn rect_from_edges(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    let (left, top) = (left.floor() as i32, top.floor() as i32);
    let (right, bottom) = (right.ceil() as i32, bottom.ceil() as i32);
    Rect::new(
        left,
        top,
        (right - left).max(0) as u32,
        (bottom - top).max(0) as u32,
    )
}

/// 把 `color` 以覆盖率 `coverage` 叠加到一个非预乘 RGBA 像素上（src-over）。
fn blend(pixel: &mut [u8], color: Color, coverage: f32) {
    let source_alpha = f32::from(color.a) / 255.0 * coverage;
    if source_alpha <= 0.0 {
        return;
    }
    if source_alpha >= 1.0 {
        pixel.copy_from_slice(&[color.r, color.g, color.b, 255]);
        return;
    }
    let target_alpha = f32::from(pixel[3]) / 255.0;
    let target_weight = target_alpha * (1.0 - source_alpha);
    let alpha = source_alpha + target_weight;
    for (channel, source) in pixel[..3].iter_mut().zip([color.r, color.g, color.b]) {
        let value =
            (f32::from(source) * source_alpha + f32::from(*channel) * target_weight) / alpha;
        *channel = value.round().clamp(0.0, 255.0) as u8;
    }
    pixel[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}
