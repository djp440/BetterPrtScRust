//! 标注文档模型：标注列表 → 合成后的截图。
//!
//! 坐标是截图的像素坐标（左上角为原点）。期望值来自几何定义（框的线条画在矩形以内、
//! 箭头沿起点到终点的线段）和逐个写出的像素，不由被测代码推导。

use std::sync::Arc;

use bps_core::{Annotation, AnnotationDocument, Stroke};
use bps_platform::{Color, Image, Point, Rect, TextMask};
use bps_platform_fake::desktop_pattern;

const RED: Color = Color::rgba(255, 0, 0, 255);

fn screenshot(width: u32, height: u32) -> Image {
    desktop_pattern(Rect::new(0, 0, width, height))
}

fn rgba(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

fn rectangle(rect: Rect, width: u32, color: Color) -> Annotation {
    Annotation::Rectangle {
        rect,
        stroke: Stroke { color, width },
    }
}

fn arrow(from: Point, to: Point, width: u32) -> Annotation {
    Annotation::Arrow {
        from,
        to,
        stroke: Stroke { color: RED, width },
    }
}

#[test]
fn empty_document_composes_to_the_screenshot_itself() {
    let base = screenshot(20, 10);

    let composed = AnnotationDocument::new().compose(&base);

    assert!(composed == base);
}

#[test]
fn rectangle_is_an_outline_of_stroke_width_just_inside_its_rect() {
    let base = screenshot(20, 16);
    let mut document = AnnotationDocument::new();
    document.add(rectangle(Rect::new(2, 3, 10, 8), 2, RED));

    let composed = document.compose(&base);

    // 外缘与矩形重合：左上角 (2,3)、右下角 (11,10) 都在线条上，线宽 2 像素。
    for (x, y) in [
        (2, 3),
        (3, 4),
        (11, 10),
        (10, 9),
        (6, 3),
        (6, 4),
        (2, 7),
        (11, 7),
    ] {
        assert_eq!(composed.pixel(x, y), rgba(RED), "({x},{y}) 应在线条上");
    }
    // 线条以内、以外都保持原样。
    for (x, y) in [
        (4, 5),
        (9, 8),
        (6, 6),
        (1, 3),
        (2, 2),
        (12, 10),
        (11, 11),
        (0, 0),
    ] {
        assert_eq!(composed.pixel(x, y), base.pixel(x, y), "({x},{y}) 不应改变");
    }
}

#[test]
fn translucent_rectangle_blends_with_the_screenshot() {
    let base = Image::from_rgba(4, 4, [0, 0, 255, 255].repeat(16));
    let mut document = AnnotationDocument::new();
    document.add(rectangle(
        Rect::new(0, 0, 4, 4),
        1,
        Color::rgba(255, 0, 0, 128),
    ));

    let composed = document.compose(&base);

    let [r, g, b, a] = composed.pixel(0, 0);
    assert!((127..=129).contains(&r) && g == 0 && (126..=128).contains(&b) && a == 255);
    assert_eq!(composed.pixel(1, 1), [0, 0, 255, 255]);
}

#[test]
fn later_annotation_is_drawn_over_earlier_one() {
    let base = screenshot(12, 12);
    let blue = Color::rgba(0, 0, 255, 255);
    let mut document = AnnotationDocument::new();
    document.add(rectangle(Rect::new(0, 0, 8, 8), 2, RED));
    document.add(rectangle(Rect::new(4, 4, 8, 8), 2, blue));

    let composed = document.compose(&base);

    // (5,7) 同时落在红框的下边和蓝框的左边上，后加的蓝框在上面。
    assert_eq!(composed.pixel(5, 7), rgba(blue));
    assert_eq!(composed.pixel(0, 0), rgba(RED));
}

#[test]
fn annotation_partly_outside_the_screenshot_is_clipped() {
    let base = screenshot(10, 10);
    let mut document = AnnotationDocument::new();
    document.add(rectangle(Rect::new(-5, -5, 10, 10), 2, RED));
    document.add(arrow(Point::new(-20, 5), Point::new(30, 5), 3));

    let composed = document.compose(&base);

    assert_eq!((composed.width(), composed.height()), (10, 10));
    assert_eq!(composed.pixel(3, 4), rgba(RED), "框的右边线");
    assert_eq!(composed.pixel(9, 5), rgba(RED), "穿过整张截图的箭头");
}

#[test]
fn arrow_shaft_runs_from_start_to_tip_with_stroke_width() {
    let base = screenshot(80, 40);
    let mut document = AnnotationDocument::new();
    document.add(arrow(Point::new(5, 20), Point::new(70, 20), 4));

    let composed = document.compose(&base);

    // 箭杆：线宽 4，中心线穿过 y=20 这一行像素的中心：19..=21 行完全覆盖，18、22 行是抗锯齿的边缘。
    for x in [6, 20, 40] {
        for y in [19, 20] {
            assert_eq!(composed.pixel(x, y), rgba(RED), "箭杆 ({x},{y})");
        }
        for y in [10, 15, 25, 30] {
            assert_eq!(composed.pixel(x, y), base.pixel(x, y), "箭杆外 ({x},{y})");
        }
    }
    // 起点之前、尖端之后都不画。
    assert_eq!(composed.pixel(2, 20), base.pixel(2, 20));
    assert_eq!(composed.pixel(74, 20), base.pixel(74, 20));
}

#[test]
fn arrow_head_at_the_tip_is_wider_than_the_shaft() {
    let base = screenshot(80, 40);
    let mut document = AnnotationDocument::new();
    document.add(arrow(Point::new(5, 20), Point::new(70, 20), 4));

    let composed = document.compose(&base);

    // 箭头尖端正指着终点像素的中心：尖端后面紧挨着的像素已经画满，尖端之外不画；
    // 靠近尖端处，箭头比箭杆宽得多。
    assert_eq!(composed.pixel(68, 20), rgba(RED), "尖端");
    assert_eq!(composed.pixel(72, 20), base.pixel(72, 20), "尖端之外");
    let head_rows = (0..40)
        .filter(|&y| composed.pixel(62, y) == rgba(RED))
        .count();
    let shaft_rows = (0..40)
        .filter(|&y| composed.pixel(20, y) == rgba(RED))
        .count();
    assert!(
        head_rows >= 2 * shaft_rows,
        "箭头 {head_rows} 行，箭杆 {shaft_rows} 行"
    );
}

#[test]
fn diagonal_arrow_covers_the_line_between_its_ends() {
    let base = screenshot(60, 60);
    let mut document = AnnotationDocument::new();
    document.add(arrow(Point::new(5, 5), Point::new(50, 50), 3));

    let composed = document.compose(&base);

    for t in [8, 15, 25, 35] {
        assert_eq!(composed.pixel(t, t), rgba(RED), "对角线上 ({t},{t})");
    }
    for (x, y) in [(20, 5), (5, 20), (40, 10), (10, 40)] {
        assert_eq!(composed.pixel(x, y), base.pixel(x, y), "远离箭头 ({x},{y})");
    }
}

fn mosaic(rect: Rect, block: u32) -> Annotation {
    Annotation::Mosaic { rect, block }
}

/// 每个像素颜色都由 `color_at(x, y)` 给出的不透明截图。
fn image_from(width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> Image {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = color_at(x, y);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image::from_rgba(width, height, pixels)
}

#[test]
fn mosaic_fills_each_block_with_the_average_color_of_that_block() {
    // 左边 4×4 一块：左两列黑、右两列白；右边 4×4 一块是纯色。
    let base = image_from(10, 6, |x, _| match x {
        0..=1 => [0, 0, 0],
        2..=3 => [255, 255, 255],
        _ => [10, 20, 30],
    });
    let mut document = AnnotationDocument::new();
    document.add(mosaic(Rect::new(0, 0, 8, 4), 4));

    let composed = document.compose(&base);

    // 黑白各一半：平均值 127.5 四舍五入为 128。
    for (x, y) in [(0, 0), (1, 3), (2, 1), (3, 3)] {
        assert_eq!(composed.pixel(x, y), [128, 128, 128, 255], "({x},{y})");
    }
    assert_eq!(composed.pixel(5, 2), [10, 20, 30, 255]);
    // 马赛克以外不变。
    for (x, y) in [(0, 4), (3, 5), (8, 0), (9, 3)] {
        assert_eq!(composed.pixel(x, y), base.pixel(x, y), "({x},{y})");
    }
}

#[test]
fn mosaic_cannot_be_reversed_because_details_inside_a_block_are_lost() {
    // 两张截图只在每个 6×6 块内部的像素排列上不同（左右镜像），块的平均颜色相同。
    let original = screenshot(36, 24);
    let mirrored = image_from(36, 24, |x, y| {
        let mirror_x = x - x % 6 + (5 - x % 6);
        let [r, g, b, _] = original.pixel(mirror_x, y);
        [r, g, b]
    });
    assert!(original != mirrored);
    let mut document = AnnotationDocument::new();
    document.add(mosaic(Rect::new(0, 0, 36, 24), 6));

    let from_original = document.compose(&original);
    let from_mirrored = document.compose(&mirrored);

    assert!(
        from_original == from_mirrored,
        "块内细节不同的两张截图打上马赛克后应完全相同"
    );
    for block_y in (0..24).step_by(6) {
        for block_x in (0..36).step_by(6) {
            let first = from_original.pixel(block_x, block_y);
            for (dx, dy) in [(5, 0), (0, 5), (3, 2), (5, 5)] {
                assert_eq!(
                    from_original.pixel(block_x + dx, block_y + dy),
                    first,
                    "块 ({block_x},{block_y}) 内应是同一种颜色"
                );
            }
        }
    }
}

#[test]
fn mosaic_edge_block_averages_the_whole_block_even_beyond_the_mosaic() {
    // 马赛克宽 5、块边长 4：第二列块只有 1 列在马赛克里，颜色仍取整个 4×4 块的平均，
    // 不会把那 1 列像素原样（或按列）泄露出来。
    let base = image_from(8, 4, |x, _| if x == 4 { [200, 0, 0] } else { [0, 0, 0] });
    let mut document = AnnotationDocument::new();
    document.add(mosaic(Rect::new(0, 0, 5, 4), 4));

    let composed = document.compose(&base);

    assert_eq!(composed.pixel(4, 0), [50, 0, 0, 255]);
    assert_eq!(composed.pixel(5, 0), [0, 0, 0, 255], "马赛克以外不变");
}

#[test]
fn mosaic_also_hides_annotations_drawn_before_it() {
    let base = image_from(16, 16, |_, _| [255, 255, 255]);
    let mut document = AnnotationDocument::new();
    // 1 像素宽的红色竖线，落在 8×8 块的第一列。
    document.add(rectangle(Rect::new(0, 0, 1, 16), 1, RED));
    document.add(mosaic(Rect::new(0, 0, 16, 16), 8));

    let composed = document.compose(&base);

    // 每块 64 个像素里有 8 个红：(255×56 + 0×8) / 64 ≈ 223。
    assert_eq!(composed.pixel(0, 0), [255, 223, 223, 255]);
    assert_eq!(composed.pixel(7, 7), [255, 223, 223, 255]);
    assert_eq!(composed.pixel(8, 0), [255, 255, 255, 255]);
}

/// 字号 2 的一个“字”：蒙版 3×2，比排版框向左多伸出 1 列。
fn tiny_text(origin: Point, color: Color) -> Annotation {
    let mask = TextMask::new(
        Rect::new(-1, 0, 3, 2),
        vec![255, 0, 128, 0, 255, 0],
        Rect::new(0, 0, 2, 2),
        Rect::new(2, 0, 1, 2),
    );
    Annotation::Text {
        origin,
        text: "x".to_owned(),
        size: 2,
        color,
        mask: Arc::new(mask),
    }
}

#[test]
fn text_is_its_glyph_coverage_painted_in_its_color_at_its_origin() {
    let base = Image::from_rgba(8, 6, [0, 0, 255, 255].repeat(48));
    let mut document = AnnotationDocument::new();
    document.add(tiny_text(Point::new(3, 2), RED));

    let composed = document.compose(&base);

    // 蒙版左上角在排版坐标 (-1,0)，即截图的 (2,2)。
    assert_eq!(composed.pixel(2, 2), rgba(RED), "完全覆盖");
    assert_eq!(composed.pixel(4, 3), [0, 0, 255, 255], "覆盖率 0");
    assert_eq!(composed.pixel(3, 3), rgba(RED));
    let [r, g, b, a] = composed.pixel(4, 2);
    assert!(
        (127..=129).contains(&r) && g == 0 && (126..=128).contains(&b) && a == 255,
        "覆盖一半时与底色各占一半：{r},{g},{b},{a}"
    );
    for (x, y) in [(1, 2), (5, 2), (3, 1), (3, 4)] {
        assert_eq!(composed.pixel(x, y), [0, 0, 255, 255], "蒙版以外 ({x},{y})");
    }
}

#[test]
fn text_partly_outside_the_screenshot_is_clipped() {
    let base = Image::from_rgba(4, 4, [0, 0, 255, 255].repeat(16));
    let mut document = AnnotationDocument::new();
    // 蒙版占截图的 x = -1..=1、y = 3..=4：左边一列和下边一行在截图以外。
    document.add(tiny_text(Point::new(0, 3), RED));

    let composed = document.compose(&base);

    assert_eq!((composed.width(), composed.height()), (4, 4));
    assert_eq!(composed.pixel(0, 3), [0, 0, 255, 255], "覆盖率 0");
    let [r, _, _, _] = composed.pixel(1, 3);
    assert!((127..=129).contains(&r), "覆盖一半：{r}");
    assert_eq!(composed.pixel(0, 2), [0, 0, 255, 255]);
}
