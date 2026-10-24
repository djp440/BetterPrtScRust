//! 长截屏的拼接：按相邻两帧之间的画面重叠，把用户竖直滚动时连续采样的帧拼成一张截图。
//!
//! 每来一帧，先和参照帧（上一次成功定位的帧）逐行比较同一位置：顶部、底部一直相同的行是
//! 这一对帧里没有滚动的部分（吸顶导航栏、吸底栏，或恰好相同的空白），只在中间的内容区里找
//! 滚动距离。找滚动距离用行的分段哈希：每行切成几段，参照帧里每个有内容（不是纯色）的分段
//! 按哈希建索引，新帧的分段到索引里查到相同内容就给“两者行号之差”投一票；得票最多的几个
//! 距离再逐行精确比较，匹配的分段足够多才采用。分段让滚动条滑块、固定的侧边栏这类只占
//! 一部分宽度的变化不至于让整行都对不上。
//!
//! 定位后只拼接新露出来的行：向下滚动时把拼接结果底部之外的新行接在后面，向上滚过开头时
//! 把新行接在前面。拼接结果最底下的几行来自当时那一帧的吸底部分，下一次向下延伸时会被
//! 真正的内容替换掉，所以吸底栏只在最后出现一次；吸顶栏同理只在最前面出现一次。
//!
//! 找不到重叠（滚得太快、两帧之间隔了超过一屏）时丢弃这一帧，参照帧不变：用户滚回能和
//! 参照帧重叠的位置后就能继续拼接，不会漏行。高度到达上限（[`LongScreenshot::MAX_HEIGHT`]）
//! 后不再接受新的帧。

use std::collections::HashMap;

use bps_platform::Image;

/// 每行切成几段分别比较。
const SEGMENTS: usize = 4;
/// 同一分段内容在参照帧里出现超过这么多次就不参与投票（例如表格里一模一样的行）。
const MAX_OCCURRENCES: usize = 8;
/// 得票最多的几个滚动距离做精确比较。
const CANDIDATES: usize = 6;
/// 精确比较时，内容相同的分段至少要占参与比较的分段的这个比例。
const MIN_MATCH_RATIO: f64 = 0.6;
/// 两帧内容区至少要重叠的行数占内容区高度的比例（另有下限 [`MIN_OVERLAP_ROWS`]）。
const MIN_OVERLAP_FRACTION: u32 = 10;
const MIN_OVERLAP_ROWS: u32 = 4;

/// 把一帧帧画面拼接成的长截屏。
///
/// 用第一帧创建，之后每采样到一帧就 [`push`](Self::push)；所有帧的尺寸必须相同
/// （长截屏区域的尺寸）。拼接结果的宽度等于帧宽，高度随滚动增长，最多
/// [`MAX_HEIGHT`](Self::MAX_HEIGHT) 像素。
pub struct LongScreenshot {
    width: u32,
    frame_height: u32,
    /// 拼接结果，RGBA，行从上到下。
    pixels: Vec<u8>,
    height: u32,
    /// 参照帧的顶边在拼接结果中的行号。
    position: i64,
    /// 拼接结果最上面这么多行来自某一帧的吸顶部分（盖住了内容），向上延伸时要被替换。
    head: u32,
    /// 拼接结果最下面这么多行来自某一帧的吸底部分，向下延伸时要被替换。
    foot: u32,
    reference: Vec<Row>,
    /// 上一次定位到的滚动距离，候选距离得分相同时优先选与它接近的。
    last_distance: i32,
    full: bool,
}

/// [`LongScreenshot::push`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stitched {
    /// 画面没有滚动（可能有原地的变化），没有新内容。
    Unchanged,
    /// 定位到这一帧相对上一帧滚动了 `distance` 行（正数表示内容向上移动，即向下滚动），
    /// 拼接结果因此多了 `added` 行（往回滚动时为 0）。
    Scrolled { distance: i32, added: u32 },
    /// 和参照帧找不到足够的重叠（例如滚动太快），这一帧被丢弃。
    Lost,
    /// 拼接结果已到达高度上限，这一帧没有用到。
    Full,
}

/// 一行的指纹：每段的哈希，以及哪些段有内容（不是纯色）。
#[derive(Clone, Copy, PartialEq, Eq)]
struct Row {
    segments: [u64; SEGMENTS],
    /// 第 k 位为 1 表示第 k 段有内容。
    informative: u8,
}

impl Row {
    fn is_informative(&self, segment: usize) -> bool {
        self.informative & (1 << segment) != 0
    }
}

impl LongScreenshot {
    /// 长截屏的最大高度（像素）。
    pub const MAX_HEIGHT: u32 = 30_000;

    /// 以开始时的画面作为第一帧。
    pub fn new(first: Image) -> Self {
        let (width, frame_height) = (first.width(), first.height());
        let reference = fingerprint(&first);
        let mut long = Self {
            width,
            frame_height,
            pixels: first.into_rgba(),
            height: frame_height,
            position: 0,
            head: 0,
            foot: 0,
            reference,
            last_distance: 0,
            full: false,
        };
        if long.height >= Self::MAX_HEIGHT {
            long.truncate(Self::MAX_HEIGHT);
            long.full = true;
        }
        long
    }

    /// 拼接结果的宽度（等于帧宽）。
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 拼接结果目前的高度。
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 高度已经到达 [`MAX_HEIGHT`](Self::MAX_HEIGHT)，不再接受新的帧。
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// 最近一次定位到的那一帧在拼接结果中占据的行（顶边、底边），可能部分超出拼接结果。
    pub fn frame_span(&self) -> (i64, i64) {
        (self.position, self.position + i64::from(self.frame_height))
    }

    /// 目前拼接出的长截屏。
    pub fn image(&self) -> Image {
        Image::from_rgba(self.width, self.height, self.pixels.clone())
    }

    /// 拼接结束，取出长截屏。
    pub fn into_image(self) -> Image {
        Image::from_rgba(self.width, self.height, self.pixels)
    }

    /// 加入新采样到的一帧。
    pub fn push(&mut self, frame: &Image) -> Stitched {
        if self.full {
            return Stitched::Full;
        }
        if (frame.width(), frame.height()) != (self.width, self.frame_height) {
            return Stitched::Lost;
        }
        let current = fingerprint(frame);
        let h = self.frame_height as usize;
        let reference = &self.reference;
        // 同一位置上一直相同的顶部、底部：这一对帧里没有滚动的部分。
        let top = (0..h).take_while(|&y| current[y] == reference[y]).count();
        if top == h {
            return Stitched::Unchanged;
        }
        let bottom = (0..h)
            .rev()
            .take_while(|&y| current[y] == reference[y])
            .count();
        let (lo, hi) = (top as u32, (h - bottom) as u32);
        let Some(distance) = self.find_distance(&current, lo, hi) else {
            return Stitched::Lost;
        };
        if distance == 0 {
            // 原地变化（例如动画），没有滚动；以后和变化后的画面比较。
            self.reference = current;
            return Stitched::Unchanged;
        }
        let Some(added) = self.place(frame, distance, lo, hi) else {
            return Stitched::Lost;
        };
        self.reference = current;
        self.last_distance = distance;
        Stitched::Scrolled { distance, added }
    }

    /// 在内容区 `[lo, hi)` 里找新帧相对参照帧的滚动距离：新帧第 `y` 行等于参照帧第 `y + d` 行。
    fn find_distance(&self, current: &[Row], lo: u32, hi: u32) -> Option<i32> {
        let reference = &self.reference;
        let mut index: HashMap<(usize, u64), Vec<u32>> = HashMap::new();
        for y in lo..hi {
            let row = &reference[y as usize];
            for k in (0..SEGMENTS).filter(|&k| row.is_informative(k)) {
                index.entry((k, row.segments[k])).or_default().push(y);
            }
        }
        let mut votes: HashMap<i32, u32> = HashMap::new();
        for y in lo..hi {
            let row = &current[y as usize];
            for k in (0..SEGMENTS).filter(|&k| row.is_informative(k)) {
                let Some(found) = index.get(&(k, row.segments[k])) else {
                    continue;
                };
                if found.len() > MAX_OCCURRENCES {
                    continue;
                }
                for &r in found {
                    *votes.entry(r as i32 - y as i32).or_default() += 1;
                }
            }
        }
        let mut ranked: Vec<(i32, u32)> = votes.into_iter().collect();
        ranked.sort_by_key(|&(d, v)| (std::cmp::Reverse(v), d.unsigned_abs(), d));
        let mut candidates: Vec<i32> = ranked.iter().take(CANDIDATES).map(|&(d, _)| d).collect();
        if !candidates.contains(&self.last_distance) {
            candidates.push(self.last_distance);
        }

        let content = hi - lo;
        let min_overlap = (content / MIN_OVERLAP_FRACTION).max(MIN_OVERLAP_ROWS);
        candidates
            .into_iter()
            .filter_map(|d| {
                let (matched, compared, overlap) = compare(reference, current, lo, hi, d);
                let enough = overlap >= min_overlap
                    && matched > 0
                    && f64::from(matched) >= MIN_MATCH_RATIO * f64::from(compared);
                enough.then_some((d, matched))
            })
            .max_by_key(|&(d, matched)| {
                (
                    matched,
                    std::cmp::Reverse((d - self.last_distance).unsigned_abs()),
                    std::cmp::Reverse(d.unsigned_abs()),
                )
            })
            .map(|(d, _)| d)
    }

    /// 按滚动距离 `distance` 把新帧放进拼接结果，接上新露出来的行；返回多出来的行数。
    /// `[lo, hi)` 是这一对帧的内容区（之外是没有滚动的吸顶、吸底部分）。
    /// 新行会用到新帧的吸顶或吸底部分（说明两帧之间漏了内容）时返回 `None`，什么也不改。
    fn place(&mut self, frame: &Image, distance: i32, lo: u32, hi: u32) -> Option<u32> {
        let h = i64::from(self.frame_height);
        let (lo, hi) = (i64::from(lo), i64::from(hi));
        let len = i64::from(self.height);
        let previous = self.position;
        let position = previous + i64::from(distance);
        let max = i64::from(Self::MAX_HEIGHT);

        if distance > 0 {
            // 参照帧的吸底部分如果正好在拼接结果最底下，那几行盖住了内容，不可信。
            let mut trusted_end = len - i64::from(self.foot);
            if previous + h == len {
                trusted_end = trusted_end.min(len - (h - hi));
            }
            if position + hi <= trusted_end {
                self.position = position;
                return Some(0);
            }
            let start = trusted_end - position;
            if start < lo {
                return None;
            }
            let mut end = h;
            if trusted_end + (end - start) > max {
                end = start + (max - trusted_end);
                self.full = true;
            }
            self.truncate(trusted_end as u32);
            self.append_rows(frame, start as u32, end as u32);
            self.foot = if self.full { 0 } else { (h - hi) as u32 };
            self.position = position;
            Some((self.height as i64 - len).max(0) as u32)
        } else {
            let mut trusted_start = i64::from(self.head);
            if previous == 0 {
                trusted_start = trusted_start.max(lo);
            }
            if position + lo >= trusted_start {
                self.position = position;
                return Some(0);
            }
            let end = trusted_start - position;
            if end > hi {
                return None;
            }
            let kept = len - trusted_start;
            let mut start = 0;
            if end + kept > max {
                start = end + kept - max;
                self.full = true;
            }
            let row = self.row_bytes();
            let mut pixels = Vec::with_capacity(((end - start + kept) as usize) * row);
            pixels.extend_from_slice(&frame.rgba()[start as usize * row..end as usize * row]);
            pixels.extend_from_slice(&self.pixels[trusted_start as usize * row..]);
            self.pixels = pixels;
            self.height = (end - start + kept) as u32;
            self.head = if start > 0 { 0 } else { lo as u32 };
            // 新帧的顶边就是拼接结果的顶边（减去被上限裁掉的部分）。
            self.position = -start;
            Some((i64::from(self.height) - len).max(0) as u32)
        }
    }

    fn row_bytes(&self) -> usize {
        self.width as usize * 4
    }

    fn truncate(&mut self, height: u32) {
        self.pixels.truncate(height as usize * self.row_bytes());
        self.height = height;
    }

    fn append_rows(&mut self, frame: &Image, from: u32, to: u32) {
        let row = self.row_bytes();
        self.pixels
            .extend_from_slice(&frame.rgba()[from as usize * row..to as usize * row]);
        self.height += to - from;
    }
}

/// 精确比较：新帧第 `y` 行与参照帧第 `y + d` 行（都在内容区 `[lo, hi)` 内）。
/// 返回（内容相同的分段数，参与比较的分段数，重叠的行数）。
fn compare(reference: &[Row], current: &[Row], lo: u32, hi: u32, d: i32) -> (u32, u32, u32) {
    let (lo, hi) = (i64::from(lo), i64::from(hi));
    let d = i64::from(d);
    let from = lo.max(lo - d);
    let to = hi.min(hi - d);
    if from >= to {
        return (0, 0, 0);
    }
    let (mut matched, mut compared) = (0, 0);
    for y in from..to {
        let (c, r) = (&current[y as usize], &reference[(y + d) as usize]);
        for k in 0..SEGMENTS {
            if c.is_informative(k) || r.is_informative(k) {
                compared += 1;
                if c.segments[k] == r.segments[k] {
                    matched += 1;
                }
            }
        }
    }
    (matched, compared, (to - from) as u32)
}

/// 每行的分段指纹。
fn fingerprint(image: &Image) -> Vec<Row> {
    let width = image.width() as usize;
    let row_bytes = width * 4;
    if row_bytes == 0 {
        return vec![
            Row {
                segments: [0; SEGMENTS],
                informative: 0
            };
            image.height() as usize
        ];
    }
    image
        .rgba()
        .chunks_exact(row_bytes)
        .map(|row| {
            let mut segments = [0; SEGMENTS];
            let mut informative = 0;
            for (k, segment) in segments.iter_mut().enumerate() {
                let from = k * width / SEGMENTS;
                let to = (k + 1) * width / SEGMENTS;
                let pixels = &row[from * 4..to * 4];
                let (hash, uniform) = hash_segment(pixels);
                *segment = hash;
                if !uniform {
                    informative |= 1 << k;
                }
            }
            Row {
                segments,
                informative,
            }
        })
        .collect()
}

/// 一段像素的哈希，以及它是不是纯色。
fn hash_segment(pixels: &[u8]) -> (u64, bool) {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut uniform = true;
    let first = pixels.get(..4);
    for pixel in pixels.chunks_exact(4) {
        let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
        hash = (hash.rotate_left(5) ^ u64::from(value)).wrapping_mul(0x517c_c1b7_2722_0a95);
        uniform &= Some(pixel) == first;
    }
    (hash, uniform)
}
