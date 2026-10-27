//! 区域采样能力：长截屏期间持续截取屏幕上一块区域的实时画面。
//!
//! 分工：`core` 决定采样哪块区域、什么时候开始和停止，并把采到的画面拼接成长截屏；
//! 平台负责在区域里的画面变化时截下最新的画面。采样在后台进行，画面有变化时由平台事件循环
//! 递交 [`PlatformEvent::RegionSampled`](crate::PlatformEvent::RegionSampled)，`core` 再用
//! [`RegionSampler::take_sample`] 取出最新的一帧；来不及处理的中间帧直接被更新的帧替换。

use crate::{Image, PlatformError, Rect};

/// 区域采样。同一时间只采样一块区域。
pub trait RegionSampler {
    /// 开始采样 `region`（虚拟桌面物理像素，可以跨越多块显示器）。正在采样时改为采样新区域。
    ///
    /// 采到的是选区遮罩底下实时的桌面：选区遮罩本身不会出现在采到的画面里。
    fn start(&mut self, region: Rect) -> Result<(), PlatformError>;

    /// 取出最近采到、还没取走的画面（尺寸等于区域）；没有新画面时返回 `Ok(None)`。
    /// 采样出了错（例如显示器被拔掉）时返回错误，之后不再有新画面。
    fn take_sample(&mut self) -> Result<Option<Image>, PlatformError>;

    /// 停止采样并释放资源；没有在采样时什么也不做。停止后不再递交新的事件
    /// （已经排进队列的仍可能递交）。
    fn stop(&mut self);
}
