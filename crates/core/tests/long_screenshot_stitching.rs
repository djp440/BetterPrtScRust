//! 长截屏拼接的公共 API（`LongScreenshot`）：用合成的帧序列模拟用户竖直滚动内容，
//! 检查按相邻帧的重叠拼出的长截屏没有重复、没有缺行。
//!
//! 帧是从一张合成的“文档”上按滚动位置裁下的一段（可以叠加固定不动的吸顶栏、吸底栏），
//! 期望结果直接从文档裁剪得到，不由被测代码推导。

use bps_core::{LongScreenshot, Stitched};
use bps_platform::Image;

const WIDTH: u32 = 48;

/// 合成的长文档：每行内容各不相同（像一行行文字），每 50 行里有 8 行空白（纯色）。
fn document(height: u32) -> Image {
    let mut pixels = Vec::with_capacity((WIDTH * height * 4) as usize);
    for y in 0..height {
        let blank = y % 50 >= 42;
        for x in 0..WIDTH {
            let pixel = if blank {
                [250, 250, 250, 255]
            } else {
                let h = mix(u64::from(x / 3) << 32 | u64::from(y));
                [h as u8, (h >> 8) as u8, (h >> 16) as u8, 255]
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    Image::from_rgba(WIDTH, height, pixels)
}

/// 吸顶栏、吸底栏这类固定不动的部分：`height` 行、颜色由 `seed` 决定、带一点图案。
fn bar(height: u32, seed: u8) -> Image {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..WIDTH {
            let edge = x % 7 == 0 || y == height - 1;
            pixels.extend_from_slice(&if edge {
                [seed, 30, 30, 255]
            } else {
                [20, seed, 200, 255]
            });
        }
    }
    Image::from_rgba(WIDTH, height, pixels)
}

fn mix(mut v: u64) -> u64 {
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51_afd7_ed55_8ccd);
    v ^= v >> 33;
    v = v.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    v ^ (v >> 33)
}

/// `image` 的第 `from` 行（含）到第 `to` 行（不含）。
fn rows(image: &Image, from: u32, to: u32) -> Image {
    let row = (image.width() * 4) as usize;
    let pixels = image.rgba()[from as usize * row..to as usize * row].to_vec();
    Image::from_rgba(image.width(), to - from, pixels)
}

/// 把几张同宽的图从上到下拼起来。
fn stack(parts: &[&Image]) -> Image {
    let pixels: Vec<u8> = parts.iter().flat_map(|p| p.rgba().to_vec()).collect();
    let height = parts.iter().map(|p| p.height()).sum();
    Image::from_rgba(WIDTH, height, pixels)
}

/// 一个滚动视口：从 `doc` 的 `offset` 行起显示 `height` 行，上下可以各有一条固定栏。
struct Viewport<'a> {
    doc: &'a Image,
    height: u32,
    header: Option<Image>,
    footer: Option<Image>,
}

impl<'a> Viewport<'a> {
    fn new(doc: &'a Image, height: u32) -> Self {
        Self {
            doc,
            height,
            header: None,
            footer: None,
        }
    }

    fn with_header(mut self, header: Image) -> Self {
        self.header = Some(header);
        self
    }

    fn with_footer(mut self, footer: Image) -> Self {
        self.footer = Some(footer);
        self
    }

    /// 内容区（两条固定栏之间）的高度。
    fn content_height(&self) -> u32 {
        self.height
            - self.header.as_ref().map_or(0, Image::height)
            - self.footer.as_ref().map_or(0, Image::height)
    }

    /// 内容滚到 `offset` 时用户看到的画面。
    fn frame(&self, offset: u32) -> Image {
        let content = rows(self.doc, offset, offset + self.content_height());
        let mut parts = Vec::new();
        parts.extend(self.header.as_ref());
        parts.push(&content);
        parts.extend(self.footer.as_ref());
        stack(&parts)
    }
}

/// 从第一个滚动位置开始，依次滚到各个位置，每个位置采样一帧，返回拼接器。
fn scroll_through(viewport: &Viewport, offsets: &[u32]) -> LongScreenshot {
    let mut long = LongScreenshot::new(viewport.frame(offsets[0]));
    for &offset in &offsets[1..] {
        long.push(&viewport.frame(offset));
    }
    long
}

fn assert_same(actual: &Image, expected: &Image) {
    assert_eq!(
        (actual.width(), actual.height()),
        (expected.width(), expected.height()),
        "长截屏尺寸不对"
    );
    let row = (WIDTH * 4) as usize;
    let first_bad = actual
        .rgba()
        .chunks_exact(row)
        .zip(expected.rgba().chunks_exact(row))
        .position(|(a, e)| a != e);
    assert_eq!(first_bad, None, "长截屏从这一行起与期望不同");
}

#[test]
fn scrolling_down_at_normal_speed_stitches_without_duplicated_or_missing_rows() {
    let doc = document(1000);
    let viewport = Viewport::new(&doc, 120);
    let offsets: Vec<u32> = (0..=600).step_by(23).collect();

    let long = scroll_through(&viewport, &offsets);

    let last = *offsets.last().unwrap();
    assert_same(&long.image(), &rows(&doc, 0, last + 120));
    assert_eq!(long.height(), last + 120);
}

#[test]
fn not_scrolling_keeps_the_first_frame() {
    let doc = document(400);
    let viewport = Viewport::new(&doc, 120);

    let mut long = LongScreenshot::new(viewport.frame(30));
    let results: Vec<Stitched> = (0..5).map(|_| long.push(&viewport.frame(30))).collect();

    assert!(
        results.iter().all(|r| *r == Stitched::Unchanged),
        "{results:?}"
    );
    assert_same(&long.image(), &rows(&doc, 30, 150));
}

#[test]
fn sticky_header_appears_only_once_at_the_top() {
    let doc = document(1000);
    let header = bar(18, 200);
    let viewport = Viewport::new(&doc, 120).with_header(header.clone());
    let offsets: Vec<u32> = (0..=500).step_by(17).collect();

    let long = scroll_through(&viewport, &offsets);

    let last = *offsets.last().unwrap();
    let content_end = last + viewport.content_height();
    assert_same(
        &long.image(),
        &stack(&[&header, &rows(&doc, 0, content_end)]),
    );
}

#[test]
fn sticky_footer_appears_only_once_at_the_bottom() {
    let doc = document(1000);
    let footer = bar(14, 90);
    let viewport = Viewport::new(&doc, 120).with_footer(footer.clone());
    let offsets: Vec<u32> = (0..=500).step_by(29).collect();

    let long = scroll_through(&viewport, &offsets);

    let last = *offsets.last().unwrap();
    let content_end = last + viewport.content_height();
    assert_same(
        &long.image(),
        &stack(&[&rows(&doc, 0, content_end), &footer]),
    );
}

#[test]
fn header_and_footer_together_each_appear_once() {
    let doc = document(1000);
    let header = bar(20, 10);
    let footer = bar(12, 160);
    let viewport = Viewport::new(&doc, 140)
        .with_header(header.clone())
        .with_footer(footer.clone());
    let offsets: Vec<u32> = (0..=420).step_by(31).collect();

    let long = scroll_through(&viewport, &offsets);

    let last = *offsets.last().unwrap();
    let content_end = last + viewport.content_height();
    assert_same(
        &long.image(),
        &stack(&[&header, &rows(&doc, 0, content_end), &footer]),
    );
}

#[test]
fn scrolling_back_up_and_down_again_does_not_duplicate_rows() {
    let doc = document(1000);
    let viewport = Viewport::new(&doc, 120);
    let mut offsets: Vec<u32> = (0..=300).step_by(25).collect();
    offsets.extend((100..300).rev().step_by(20));
    offsets.extend((100..=520).step_by(30));

    let long = scroll_through(&viewport, &offsets);

    assert_same(&long.image(), &rows(&doc, 0, 520 + 120));
}

#[test]
fn scrolling_up_past_the_starting_point_adds_rows_above() {
    let doc = document(1000);
    let header = bar(16, 70);
    let viewport = Viewport::new(&doc, 120).with_header(header.clone());
    let mut offsets: Vec<u32> = (0..=300).rev().step_by(20).map(|o| o + 100).collect();
    offsets.extend((100..=500).step_by(40));

    let long = scroll_through(&viewport, &offsets);

    let content_end = 500 + viewport.content_height();
    assert_eq!(offsets.iter().min(), Some(&100));
    assert_same(
        &long.image(),
        &stack(&[&header, &rows(&doc, 100, content_end)]),
    );
}

#[test]
fn frames_that_scrolled_too_far_are_dropped_until_the_content_overlaps_again() {
    let doc = document(1000);
    let viewport = Viewport::new(&doc, 120);

    let mut long = LongScreenshot::new(viewport.frame(0));
    assert!(matches!(
        long.push(&viewport.frame(60)),
        Stitched::Scrolled { distance: 60, .. }
    ));
    // 一下子滚过了一整屏：和上一帧没有重叠，丢弃。
    assert_eq!(long.push(&viewport.frame(300)), Stitched::Lost);
    assert_eq!(long.push(&viewport.frame(420)), Stitched::Lost);
    // 用户往回滚到还能和上一帧重叠的位置，继续拼接。
    assert!(matches!(
        long.push(&viewport.frame(150)),
        Stitched::Scrolled { distance: 90, .. }
    ));
    for offset in (150..=400).step_by(50) {
        long.push(&viewport.frame(offset));
    }

    assert_same(&long.image(), &rows(&doc, 0, 400 + 120));
}

#[test]
fn fast_scrolling_with_little_overlap_still_stitches_correctly() {
    let doc = document(2000);
    let viewport = Viewport::new(&doc, 120);
    // 每次滚动 100 行，只剩 20 行重叠。
    let offsets: Vec<u32> = (0..=1500).step_by(100).collect();

    let long = scroll_through(&viewport, &offsets);

    assert_same(&long.image(), &rows(&doc, 0, 1500 + 120));
}

#[test]
fn moving_scrollbar_thumb_at_the_right_edge_does_not_break_stitching() {
    let doc = document(1000);
    let viewport = Viewport::new(&doc, 120);
    // 最右边 3 列是滚动条：滑块（深色）随滚动位置下移，其余是浅色轨道。
    let with_scrollbar = |offset: u32| {
        let frame = viewport.frame(offset);
        let mut pixels = frame.into_rgba();
        let thumb = offset * 120 / 1000;
        for y in 0..120 {
            for x in WIDTH - 3..WIDTH {
                let dark = (thumb..thumb + 14).contains(&y);
                let color = if dark {
                    [90, 90, 90, 255]
                } else {
                    [235, 235, 235, 255]
                };
                let i = ((y * WIDTH + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&color);
            }
        }
        Image::from_rgba(WIDTH, 120, pixels)
    };
    let offsets: Vec<u32> = (0..=600).step_by(21).collect();

    let mut long = LongScreenshot::new(with_scrollbar(0));
    for &offset in &offsets[1..] {
        long.push(&with_scrollbar(offset));
    }

    // 只比较滚动条以外的内容。
    let image = long.image();
    let end = offsets.last().unwrap() + 120;
    assert_eq!(image.height(), end);
    let expected = rows(&doc, 0, end);
    for y in 0..image.height() {
        for x in 0..WIDTH - 3 {
            assert_eq!(image.pixel(x, y), expected.pixel(x, y), "({x}, {y})");
        }
    }
}

#[test]
fn stitching_stops_at_the_height_limit() {
    let doc = document(LongScreenshot::MAX_HEIGHT + 2000);
    let viewport = Viewport::new(&doc, 200);
    let mut long = LongScreenshot::new(viewport.frame(0));
    let mut offset = 0;
    let mut last = Stitched::Unchanged;
    while !long.is_full() && offset < doc.height() - 200 {
        offset += 150;
        last = long.push(&viewport.frame(offset.min(doc.height() - 200)));
    }

    assert!(long.is_full());
    assert!(matches!(last, Stitched::Scrolled { .. }));
    assert_eq!(long.push(&viewport.frame(offset + 50)), Stitched::Full);
    assert_same(&long.image(), &rows(&doc, 0, LongScreenshot::MAX_HEIGHT));
}

#[test]
fn irregular_scroll_steps_from_smooth_scrolling_stitch_correctly() {
    let doc = document(2000);
    let header = bar(10, 33);
    let viewport = Viewport::new(&doc, 100).with_header(header.clone());
    let steps = [1, 2, 7, 40, 61, 3, 1, 0, 25, 77, 12, 5, 0, 0, 33, 80, 2];
    let mut offsets = vec![0];
    for step in steps.iter().cycle().take(60) {
        offsets.push(offsets.last().unwrap() + step);
    }

    let long = scroll_through(&viewport, &offsets);

    let content_end = offsets.last().unwrap() + viewport.content_height();
    assert_same(
        &long.image(),
        &stack(&[&header, &rows(&doc, 0, content_end)]),
    );
}

/// `bar` 上一小块变了色：像鼠标悬停高亮了工具栏按钮、任务栏时钟跳了一分钟。
fn highlighted(bar: &Image) -> Image {
    let mut pixels = bar.rgba().to_vec();
    for y in 2..bar.height() - 2 {
        for x in 30..40 {
            let i = ((y * WIDTH + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&[255, 220, 0, 255]);
        }
    }
    Image::from_rgba(WIDTH, bar.height(), pixels)
}

#[test]
fn small_in_place_change_in_a_fixed_bar_is_not_mistaken_for_scrolling_too_fast() {
    let doc = document(1000);
    let toolbar = bar(18, 200);
    let hovered = highlighted(&toolbar);
    let before = Viewport::new(&doc, 120).with_header(toolbar.clone());
    let after = Viewport::new(&doc, 120).with_header(hovered.clone());

    let mut long = LongScreenshot::new(before.frame(0));
    assert!(matches!(
        long.push(&before.frame(40)),
        Stitched::Scrolled { distance: 40, .. }
    ));
    // 没有滚动，只有工具栏上的按钮变了高亮。
    assert_eq!(long.push(&after.frame(40)), Stitched::Unchanged);
    assert_eq!(long.push(&before.frame(40)), Stitched::Unchanged);
    for offset in (40..=400).step_by(30) {
        long.push(&after.frame(offset));
    }

    let content_end = 400 + before.content_height();
    assert_same(
        &long.image(),
        &stack(&[&toolbar, &rows(&doc, 0, content_end)]),
    );
}

#[test]
fn small_in_place_change_in_a_fixed_bottom_bar_is_not_mistaken_for_scrolling_too_fast() {
    let doc = document(1000);
    let taskbar = bar(14, 90);
    let ticked = highlighted(&taskbar);
    let before = Viewport::new(&doc, 120).with_footer(taskbar.clone());
    let after = Viewport::new(&doc, 120).with_footer(ticked.clone());

    let mut long = LongScreenshot::new(before.frame(0));
    assert_eq!(long.push(&after.frame(0)), Stitched::Unchanged);
    for offset in (0..=400).step_by(30) {
        long.push(&after.frame(offset));
    }

    let content_end = 390 + after.content_height();
    assert_same(
        &long.image(),
        &stack(&[&rows(&doc, 0, content_end), &ticked]),
    );
}
