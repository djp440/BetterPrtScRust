//! 用 Windows.Graphics.Capture 持续捕获虚拟桌面上的一块区域。
//!
//! 区域经过的每块显示器各开一个捕获会话（自由线程的帧池，不需要调度队列）。`latest` 被调用时
//! 取出各会话最新的一帧，按区域裁剪拼进一张新的画布纹理；画面没变时（系统只在画面或指针变化时
//! 才给新帧）沿用上一张画布。显示器之间的空隙是黑色。
//!
//! 刚开始时先等各显示器的第一帧都到齐（最多 [`FIRST_FRAME_WAIT`]），避免开头几帧缺一块。
//! 显示器被拔掉或分辨率变了时，流就结束了（[`StreamFrame::Ended`]）。

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bps_platform::{
    Display, FrameSource, PlatformError, Rect, StreamFrame, StreamOptions, VideoFrame, VideoStream,
};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface};

use super::{TextureFrame, VideoDevice};
use crate::error::win_error;
use crate::frame_source::DxgiFrameSource;

/// 开始捕获后最多等多久让各显示器的第一帧到齐；超时后缺的部分先用黑色。
const FIRST_FRAME_WAIT: Duration = Duration::from_millis(500);
/// 每个帧池的缓冲数。
const POOL_BUFFERS: i32 = 2;

pub(super) struct RegionStream {
    device: Rc<VideoDevice>,
    region: Rect,
    monitors: Vec<MonitorCapture>,
    latest: Option<Rc<TextureFrame>>,
    opened: Instant,
}

impl RegionStream {
    pub fn open(
        device: Rc<VideoDevice>,
        region: Rect,
        options: StreamOptions,
    ) -> Result<Self, PlatformError> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(PlatformError::new(
                "当前系统不支持 Windows.Graphics.Capture，无法录屏（需要 Windows 10 1903 或更高版本）",
            ));
        }
        let monitors = DxgiFrameSource::new()
            .displays()?
            .into_iter()
            .filter(|display| display.bounds.intersection(region).is_some())
            .map(|display| MonitorCapture::start(&device, &display, options))
            .collect::<Result<Vec<_>, _>>()?;
        if monitors.is_empty() {
            return Err(PlatformError::new("录屏区域不在任何显示器上"));
        }
        Ok(Self {
            device,
            region,
            monitors,
            latest: None,
            opened: Instant::now(),
        })
    }

    /// 把各显示器已到的新帧拼进一张新画布。
    fn compose(&mut self) -> Result<Rc<TextureFrame>, PlatformError> {
        let canvas = self.create_canvas()?;
        let context = &self.device.context;
        unsafe {
            match &self.latest {
                // 没有新帧的显示器（和显示器之间的空隙）沿用上一张画布上的画面。
                Some(previous) => context.CopyResource(&canvas, &previous.texture),
                None => {
                    let mut view: Option<ID3D11RenderTargetView> = None;
                    self.device
                        .d3d
                        .CreateRenderTargetView(&canvas, None, Some(&mut view))
                        .map_err(|e| win_error("创建录屏画布视图失败", e))?;
                    if let Some(view) = view {
                        context.ClearRenderTargetView(&view, &[0.0, 0.0, 0.0, 1.0]);
                    }
                }
            }
        }
        for monitor in &mut self.monitors {
            let Some(frame) = monitor.pending.take() else {
                continue;
            };
            let copied = copy_into(&self.device, &canvas, self.region, monitor.bounds, &frame);
            let _ = frame.Close();
            copied?;
            monitor.received = true;
        }
        Ok(Rc::new(TextureFrame {
            device: self.device.clone(),
            texture: canvas,
            width: self.region.width,
            height: self.region.height,
        }))
    }

    fn create_canvas(&self) -> Result<ID3D11Texture2D, PlatformError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.region.width,
            Height: self.region.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe {
            self.device
                .d3d
                .CreateTexture2D(&desc, None, Some(&mut texture))
        }
        .map_err(|e| win_error("创建录屏画布失败", e))?;
        texture.ok_or_else(|| PlatformError::new("创建录屏画布失败"))
    }
}

impl VideoStream for RegionStream {
    fn latest(&mut self) -> Result<StreamFrame, PlatformError> {
        for monitor in &mut self.monitors {
            if !monitor.poll()? {
                return Ok(StreamFrame::Ended);
            }
        }
        let arrived = self.monitors.iter().any(|m| m.pending.is_some());
        let complete = self
            .monitors
            .iter()
            .all(|m| m.received || m.pending.is_some());
        let compose = match &self.latest {
            Some(_) => arrived,
            None => arrived && (complete || self.opened.elapsed() >= FIRST_FRAME_WAIT),
        };
        if compose {
            self.latest = Some(self.compose()?);
        }
        Ok(match &self.latest {
            Some(frame) => StreamFrame::Frame(frame.clone() as Rc<dyn VideoFrame>),
            None => StreamFrame::Pending,
        })
    }
}

/// 一块显示器上的捕获会话。
struct MonitorCapture {
    /// 显示器在虚拟桌面中的范围（物理像素）。
    bounds: Rect,
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// 捕获目标已关闭（显示器被拔掉等），由系统在其他线程上设置。
    closed: Arc<AtomicBool>,
    closed_token: i64,
    /// 已经到达、还没拼进画布的最新一帧。
    pending: Option<Direct3D11CaptureFrame>,
    /// 已经拼进过画布。
    received: bool,
}

impl MonitorCapture {
    fn start(
        device: &VideoDevice,
        display: &Display,
        options: StreamOptions,
    ) -> Result<Self, PlatformError> {
        let monitor = HMONITOR(display.id.0 as usize as *mut _);
        let item: GraphicsCaptureItem =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .and_then(|interop| unsafe { interop.CreateForMonitor(monitor) })
                .map_err(|e| win_error("创建显示器的捕获目标失败", e))?;
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let closed_token = item
            .Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    flag.store(true, Ordering::Relaxed);
                    Ok(())
                }),
            )
            .map_err(|e| win_error("监听捕获目标失败", e))?;
        let size = item
            .Size()
            .map_err(|e| win_error("读取捕获目标尺寸失败", e))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &device.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            size,
        )
        .map_err(|e| win_error("创建捕获帧池失败", e))?;
        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| win_error("创建捕获会话失败", e))?;
        // 以下两项在较旧的系统上不支持：鼠标指针开关需要 Windows 10 2004，
        // 去掉黄色捕获边框需要 Windows 11。不支持时沿用系统默认。
        let _ = session.SetIsCursorCaptureEnabled(options.show_cursor);
        let _ = session.SetIsBorderRequired(false);
        session
            .StartCapture()
            .map_err(|e| win_error("开始捕获失败", e))?;
        Ok(Self {
            bounds: display.bounds,
            item,
            pool,
            session,
            closed,
            closed_token,
            pending: None,
            received: false,
        })
    }

    /// 取出帧池里已到的全部帧，只留最新的一帧。返回捕获目标是否还在。
    fn poll(&mut self) -> Result<bool, PlatformError> {
        if self.closed.load(Ordering::Relaxed) {
            return Ok(false);
        }
        loop {
            let frame = match self.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                // 没有新帧时返回空指针。
                Err(e) if e.code().is_ok() => return Ok(true),
                Err(e) => return Err(win_error("读取捕获帧失败", e)),
            };
            let size = frame
                .ContentSize()
                .map_err(|e| win_error("读取捕获帧尺寸失败", e))?;
            if size.Width != self.bounds.width as i32 || size.Height != self.bounds.height as i32 {
                // 分辨率或缩放方向变了：显示器布局已经不是开始录制时的样子。
                let _ = frame.Close();
                return Ok(false);
            }
            if let Some(older) = self.pending.replace(frame) {
                let _ = older.Close();
            }
        }
    }
}

impl Drop for MonitorCapture {
    fn drop(&mut self) {
        if let Some(frame) = self.pending.take() {
            let _ = frame.Close();
        }
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// 把显示器 `bounds` 上的一帧中落在 `region` 里的部分拷到画布的对应位置。
fn copy_into(
    device: &VideoDevice,
    canvas: &ID3D11Texture2D,
    region: Rect,
    bounds: Rect,
    frame: &Direct3D11CaptureFrame,
) -> Result<(), PlatformError> {
    let Some(overlap) = region.intersection(bounds) else {
        return Ok(());
    };
    let texture: ID3D11Texture2D = frame
        .Surface()
        .and_then(|surface| surface.cast::<IDirect3DDxgiInterfaceAccess>())
        .and_then(|access| unsafe { access.GetInterface() })
        .map_err(|e| win_error("读取捕获帧纹理失败", e))?;
    let left = (overlap.x - bounds.x) as u32;
    let top = (overlap.y - bounds.y) as u32;
    let source = D3D11_BOX {
        left,
        top,
        front: 0,
        right: left + overlap.width,
        bottom: top + overlap.height,
        back: 1,
    };
    unsafe {
        device.context.CopySubresourceRegion(
            canvas,
            0,
            (overlap.x - region.x) as u32,
            (overlap.y - region.y) as u32,
            0,
            &texture,
            0,
            Some(&source),
        );
    }
    Ok(())
}
