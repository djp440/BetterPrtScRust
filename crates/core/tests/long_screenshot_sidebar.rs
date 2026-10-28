//! 长截屏拼接的固定侧栏（`LongScreenshot`）：左侧或右侧边缘有一条滚动时不动的纵向侧栏
//! （浏览器垂直标签栏、网页固定侧栏）时，侧栏只出现一次：第一帧的内容放在长图最顶部，
//! 其下的延长区域用侧栏的主要背景色填充，正文照常拼接，宽度不变。
//!
//! 帧由“固定侧栏 + 滚动的文档 +（可选）吸顶栏、吸底栏”合成，期望结果由测试自己按上述
//! 输出规则从文档和第一帧裁剪得到，不由被测代码推导。

use bps_core::LongScreenshot;
use bps_platform::Image;

const SIDEBAR: u32 = 16;
const CONTENT: u32 = 48;
const WIDTH: u32 = SIDEBAR + CONTENT;
const FRAME_HEIGHT: u32 = 120;
const BACKGROUND: [u8; 4] = [30, 30, 40, 255];

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Left,
    Right,
    None,
}

fn mix(mut v: u64) -> u64 {
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51_afd7_ed55_8ccd);
    v ^= v >> 33;
    v = v.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    v ^ (v >> 33)
}

/// 每行内容各不相同的文档（宽 `CONTENT`），每 50 行里有 8 行纯色空白。
fn document(height: u32) -> Image {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..CONTENT {
            pixels.extend_from_slice(&if y % 50 >= 42 {
                [250, 250, 250, 255]
            } else {
                let h = mix(u64::from(x / 3) << 32 | u64::from(y));
                [h as u8, (h >> 8) as u8, (h >> 16) as u8, 255]
            });
        }
    }
    Image::from_rgba(CONTENT, height, pixels)
}

/// 侧栏：大部分是背景色，每 10 行有 3 行带“图标”，每个图标颜色各不相同。
fn sidebar() -> Image {
    let mut pixels = Vec::new();
    for y in 0..FRAME_HEIGHT {
        for x in 0..SIDEBAR {
            pixels.extend_from_slice(&if y % 10 < 3 && (4..12).contains(&x) {
                let h = mix(u64::from(y));
                [h as u8, (h >> 8) as u8, 255, 255]
            } else {
                BACKGROUND
            });
        }
    }
    Image::from_rgba(SIDEBAR, FRAME_HEIGHT, pixels)
}

/// 吸顶、吸底栏：横跨整个宽度（包括侧栏所在的列）。
fn bar(height: u32, seed: u8) -> Image {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..WIDTH {
            pixels.extend_from_slice(&if x % 7 == 0 || y == height - 1 {
                [seed, 30, 30, 255]
            } else {
                [20, seed, 200, 255]
            });
        }
    }
    Image::from_rgba(WIDTH, height, pixels)
}

fn rows(image: &Image, from: u32, to: u32) -> Image {
    let row = (image.width() * 4) as usize;
    Image::from_rgba(
        image.width(),
        to - from,
        image.rgba()[from as usize * row..to as usize * row].to_vec(),
    )
}

fn columns(image: &Image, from: u32, to: u32) -> Image {
    let row = image.width() as usize * 4;
    let pixels = image
        .rgba()
        .chunks_exact(row)
        .flat_map(|r| r[from as usize * 4..to as usize * 4].to_vec())
        .collect();
    Image::from_rgba(to - from, image.height(), pixels)
}

fn stack(parts: &[&Image]) -> Image {
    let pixels: Vec<u8> = parts.iter().flat_map(|p| p.rgba().to_vec()).collect();
    Image::from_rgba(
        parts[0].width(),
        parts.iter().map(|p| p.height()).sum(),
        pixels,
    )
}

/// 把 `left` 和 `right` 左右并排（同高）。
fn beside(left: &Image, right: &Image) -> Image {
    let mut pixels = Vec::new();
    for (l, r) in left
        .rgba()
        .chunks_exact(left.width() as usize * 4)
        .zip(right.rgba().chunks_exact(right.width() as usize * 4))
    {
        pixels.extend_from_slice(l);
        pixels.extend_from_slice(r);
    }
    Image::from_rgba(left.width() + right.width(), left.height(), pixels)
}

/// 滚动视口：固定侧栏在 `side`，上下可以各有一条横跨全宽的固定栏。
struct Viewport {
    doc: Image,
    side: Side,
    header: Option<Image>,
    footer: Option<Image>,
}

impl Viewport {
    fn new(doc: Image, side: Side) -> Self {
        Self {
            doc,
            side,
            header: None,
            footer: None,
        }
    }

    fn header_height(&self) -> u32 {
        self.header.as_ref().map_or(0, Image::height)
    }

    fn footer_height(&self) -> u32 {
        self.footer.as_ref().map_or(0, Image::height)
    }

    fn content_height(&self) -> u32 {
        FRAME_HEIGHT - self.header_height() - self.footer_height()
    }

    /// 正文列的宽度（没有侧栏时是整幅宽度）。
    fn main_width(&self) -> u32 {
        if self.side == Side::None {
            WIDTH
        } else {
            CONTENT
        }
    }

    fn frame(&self, offset: u32) -> Image {
        let (header_h, footer_h) = (self.header_height(), self.footer_height());
        let blank = |h: u32| {
            Image::from_rgba(
                self.main_width(),
                h,
                vec![0; (self.main_width() * h * 4) as usize],
            )
        };
        let content = rows(&self.doc, offset, offset + self.content_height());
        let main = stack(&[&blank(header_h), &content, &blank(footer_h)]);
        let body = match self.side {
            Side::Left => beside(&sidebar(), &main),
            Side::Right => beside(&main, &sidebar()),
            Side::None => main,
        };
        // 吸顶、吸底栏盖在整个宽度（包括侧栏所在的列）上。
        let mut parts = Vec::new();
        parts.extend(self.header.clone());
        parts.push(rows(&body, header_h, FRAME_HEIGHT - footer_h));
        parts.extend(self.footer.clone());
        stack(&parts.iter().collect::<Vec<_>>())
    }

    /// 期望的长截屏：正文是 `[from, to)` 行文档（加上两条固定栏的正文部分），侧栏按规则放置。
    fn expected(&self, first_offset: u32, from: u32, to: u32) -> Image {
        let first = self.frame(first_offset);
        let (main_from, main_to) = match self.side {
            Side::Left => (SIDEBAR, WIDTH),
            Side::Right => (0, CONTENT),
            Side::None => (0, WIDTH),
        };
        let mut parts = Vec::new();
        let (head, foot);
        if let Some(header) = &self.header {
            head = columns(header, main_from, main_to);
            parts.push(&head);
        }
        let content = rows(&self.doc, from, to);
        parts.push(&content);
        if let Some(footer) = &self.footer {
            foot = columns(footer, main_from, main_to);
            parts.push(&foot);
        }
        let main = stack(&parts);
        if self.side == Side::None {
            return main;
        }
        let bar_columns = match self.side {
            Side::Left => (0, SIDEBAR),
            _ => (CONTENT, WIDTH),
        };
        let snapshot = columns(&first, bar_columns.0, bar_columns.1);
        let fill = Image::from_rgba(
            SIDEBAR,
            main.height() - FRAME_HEIGHT,
            BACKGROUND.repeat((SIDEBAR * (main.height() - FRAME_HEIGHT)) as usize),
        );
        let side = stack(&[&snapshot, &fill]);
        match self.side {
            Side::Left => beside(&side, &main),
            _ => beside(&main, &side),
        }
    }
}

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
    let row = actual.width() as usize * 4;
    let first_bad = actual
        .rgba()
        .chunks_exact(row)
        .zip(expected.rgba().chunks_exact(row))
        .position(|(a, e)| a != e);
    assert_eq!(first_bad, None, "长截屏从这一行起与期望不同");
}

#[test]
fn left_sidebar_appears_once_at_the_top_when_scrolling_down() {
    let viewport = Viewport::new(document(1000), Side::Left);
    let offsets: Vec<u32> = (0..=390).step_by(30).collect();

    let long = scroll_through(&viewport, &offsets);

    assert_same(&long.image(), &viewport.expected(0, 0, 390 + FRAME_HEIGHT));
}

#[test]
fn right_sidebar_appears_once_at_the_top_when_scrolling_down() {
    let viewport = Viewport::new(document(1000), Side::Right);
    let offsets: Vec<u32> = (0..=390).step_by(30).collect();

    let long = scroll_through(&viewport, &offsets);

    assert_same(&long.image(), &viewport.expected(0, 0, 390 + FRAME_HEIGHT));
}

#[test]
fn sidebar_stays_at_the_top_when_scrolling_up_past_the_start() {
    let viewport = Viewport::new(document(1000), Side::Left);
    // 从中途出发先向上延伸，再向下。
    let mut offsets: Vec<u32> = (0..=300).rev().step_by(25).map(|o| o + 100).collect();
    offsets.extend((100..=400).step_by(30));

    let long = scroll_through(&viewport, &offsets);

    assert_same(
        &long.image(),
        &viewport.expected(offsets[0], 100, 400 + FRAME_HEIGHT),
    );
}

#[test]
fn scrolling_back_and_forth_gives_the_same_layout() {
    let viewport = Viewport::new(document(1000), Side::Left);
    let mut offsets: Vec<u32> = (0..=300).step_by(25).collect();
    offsets.extend((100..300).rev().step_by(20));
    offsets.extend((100..=520).step_by(30));

    let long = scroll_through(&viewport, &offsets);

    assert_same(&long.image(), &viewport.expected(0, 0, 520 + FRAME_HEIGHT));
}

#[test]
fn sidebar_crossing_sticky_header_and_footer_follows_the_same_rule() {
    let mut viewport = Viewport::new(document(1000), Side::Left);
    viewport.header = Some(bar(16, 70));
    viewport.footer = Some(bar(12, 160));
    let offsets: Vec<u32> = (0..=400).step_by(25).collect();

    let long = scroll_through(&viewport, &offsets);

    let end = 400 + viewport.content_height();
    assert_same(&long.image(), &viewport.expected(0, 0, end));
}

#[test]
fn blank_margin_next_to_content_is_not_mistaken_for_a_sidebar() {
    // 左边 16 列是不变的纯色空白边距，右边是滚动的文档：整幅都是正文，没有侧栏。
    let doc = document(1000);
    let margin = |h: u32| Image::from_rgba(SIDEBAR, h, BACKGROUND.repeat((SIDEBAR * h) as usize));
    let frame = |offset: u32| {
        beside(
            &margin(FRAME_HEIGHT),
            &rows(&doc, offset, offset + FRAME_HEIGHT),
        )
    };
    let mut long = LongScreenshot::new(frame(0));
    for offset in (30..=300).step_by(30) {
        long.push(&frame(offset));
    }

    let expected = beside(
        &margin(300 + FRAME_HEIGHT),
        &rows(&doc, 0, 300 + FRAME_HEIGHT),
    );
    assert_same(&long.image(), &expected);
}

#[test]
fn page_without_sidebar_is_unaffected() {
    let viewport = Viewport::new(document(1000), Side::None);
    // 没有侧栏时文档宽度应等于整幅宽度：用同样的合成方式重做一份宽文档。
    assert_eq!(viewport.main_width(), WIDTH);
    let wide: Vec<u8> = (0..1000u32)
        .flat_map(|y| {
            (0..WIDTH).flat_map(move |x| {
                let h = mix(u64::from(x / 3) << 32 | u64::from(y));
                [h as u8, (h >> 8) as u8, (h >> 16) as u8, 255]
            })
        })
        .collect();
    let doc = Image::from_rgba(WIDTH, 1000, wide);
    let mut long = LongScreenshot::new(rows(&doc, 0, FRAME_HEIGHT));
    for offset in (30..=300).step_by(30) {
        long.push(&rows(&doc, offset, offset + FRAME_HEIGHT));
    }

    assert_same(&long.image(), &rows(&doc, 0, 300 + FRAME_HEIGHT));
}
