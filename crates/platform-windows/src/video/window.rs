//! 用 Windows.Graphics.Capture 持续捕获一个顶层窗口（窗口录屏）。
//!
//! 捕获的是窗口自己的内容（DWM 的重定向表面），被其他窗口遮挡、超出屏幕的部分也完整，
//! 不含阴影。输出画布的尺寸在打开时固定（编码器要求每帧尺寸一致）：每次有新帧，就新建一张
//! 画布纹理，涂黑，再把窗口内容贴在左上角——窗口之后变大时超出的部分被裁掉，变小时其余部分是
//! 黑色，不缩放。窗口改变大小时帧池按新尺寸重建（系统要求），录制不中断。
//! 窗口关闭（或句柄失效）后流结束（[`StreamFrame::Ended`]）。

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bps_platform::{PlatformError, StreamFrame, StreamOptions, VideoFrame, VideoStream, WindowId};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;
use windows::core::{IInspectable, Interface};

use super::{TextureFrame, VideoDevice};
use crate::error::win_error;
use crate::window_source::hwnd_of;

const POOL_BUFFERS: i32 = 2;
const PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

pub(super) struct WindowStream {
    device: Rc<VideoDevice>,
    hwnd: HWND,
    width: u32,
    height: u32,
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// 帧池当前的缓冲尺寸；窗口内容尺寸与它不同时重建帧池。
    pool_size: SizeInt32,
    /// 窗口已关闭，由系统在其他线程上设置。
    closed: Arc<AtomicBool>,
    closed_token: i64,
    latest: Option<Rc<TextureFrame>>,
}

impl WindowStream {
    pub fn open(
        device: Rc<VideoDevice>,
        window: WindowId,
        width: u32,
        height: u32,
        options: StreamOptions,
    ) -> Result<Self, PlatformError> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(PlatformError::new(
                "当前系统不支持 Windows.Graphics.Capture，无法录屏（需要 Windows 10 1903 或更高版本）",
            ));
        }
        let hwnd = hwnd_of(window)?;
        let item: GraphicsCaptureItem =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .and_then(|interop| unsafe { interop.CreateForWindow(hwnd) })
                .map_err(|e| win_error("无法捕获这个窗口", e))?;
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let closed_token = item
            .Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    flag.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            )
            .map_err(|e| win_error("监听窗口关闭失败", e))?;
        let pool_size = item.Size().map_err(|e| win_error("读取窗口尺寸失败", e))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &device.winrt,
            PIXEL_FORMAT,
            POOL_BUFFERS,
            pool_size,
        )
        .map_err(|e| win_error("创建捕获帧池失败", e))?;
        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| win_error("创建捕获会话失败", e))?;
        // 鼠标指针开关需要 Windows 10 2004，去掉黄色边框需要 Windows 11；不支持时沿用系统默认。
        let _ = session.SetIsCursorCaptureEnabled(options.show_cursor);
        let _ = session.SetIsBorderRequired(false);
        session
            .StartCapture()
            .map_err(|e| win_error("开始捕获窗口失败", e))?;
        Ok(Self {
            device,
            hwnd,
            width,
            height,
            item,
            pool,
            session,
            pool_size,
            closed,
            closed_token,
            latest: None,
        })
    }

    /// 取出帧池里最新的一帧并贴进新画布；没有新帧时返回 `None`。
    fn take_new_frame(&mut self) -> Result<Option<Rc<TextureFrame>>, PlatformError> {
        let mut newest = None;
        loop {
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    if let Some(older) = newest.replace(frame) {
                        let _ = older.Close();
                    }
                }
                // 没有新帧时返回空指针。
                Err(e) if e.code().is_ok() => break,
                Err(e) => return Err(win_error("读取捕获帧失败", e)),
            }
        }
        let Some(frame) = newest else {
            return Ok(None);
        };
        let content = frame
            .ContentSize()
            .map_err(|e| win_error("读取捕获帧尺寸失败", e))?;
        if (content.Width, content.Height) != (self.pool_size.Width, self.pool_size.Height)
            && content.Width > 0
            && content.Height > 0
        {
            // 窗口改变了大小：帧池按新尺寸重建，之后的帧是新尺寸。
            self.pool
                .Recreate(&self.device.winrt, PIXEL_FORMAT, POOL_BUFFERS, content)
                .map_err(|e| win_error("重建捕获帧池失败", e))?;
            self.pool_size = content;
        }
        let canvas = self.create_canvas()?;
        let pasted = self.paste(&canvas, &frame, content);
        let _ = frame.Close();
        pasted?;
        Ok(Some(Rc::new(TextureFrame {
            device: self.device.clone(),
            texture: canvas,
            width: self.width,
            height: self.height,
        })))
    }

    /// 新建一张涂黑的画布。
    fn create_canvas(&self) -> Result<ID3D11Texture2D, PlatformError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
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
        let texture = texture.ok_or_else(|| PlatformError::new("创建录屏画布失败"))?;
        let mut view: Option<ID3D11RenderTargetView> = None;
        unsafe {
            self.device
                .d3d
                .CreateRenderTargetView(&texture, None, Some(&mut view))
                .map_err(|e| win_error("创建录屏画布视图失败", e))?;
            if let Some(view) = view {
                self.device
                    .context
                    .ClearRenderTargetView(&view, &[0.0, 0.0, 0.0, 1.0]);
            }
        }
        Ok(texture)
    }

    /// 把窗口内容贴到画布左上角，超出画布的部分裁掉。
    fn paste(
        &self,
        canvas: &ID3D11Texture2D,
        frame: &Direct3D11CaptureFrame,
        content: SizeInt32,
    ) -> Result<(), PlatformError> {
        let texture: ID3D11Texture2D = frame
            .Surface()
            .and_then(|surface| surface.cast::<IDirect3DDxgiInterfaceAccess>())
            .and_then(|access| unsafe { access.GetInterface() })
            .map_err(|e| win_error("读取捕获帧纹理失败", e))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let right = (content.Width.max(0) as u32)
            .min(self.width)
            .min(desc.Width);
        let bottom = (content.Height.max(0) as u32)
            .min(self.height)
            .min(desc.Height);
        if right == 0 || bottom == 0 {
            return Ok(());
        }
        let source = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right,
            bottom,
            back: 1,
        };
        unsafe {
            self.device.context.CopySubresourceRegion(
                canvas,
                0,
                0,
                0,
                0,
                &texture,
                0,
                Some(&source),
            );
        }
        Ok(())
    }
}

impl VideoStream for WindowStream {
    fn latest(&mut self) -> Result<StreamFrame, PlatformError> {
        if self.closed.load(Ordering::SeqCst) || !unsafe { IsWindow(Some(self.hwnd)) }.as_bool() {
            return Ok(StreamFrame::Ended);
        }
        if let Some(frame) = self.take_new_frame()? {
            self.latest = Some(frame);
        }
        Ok(match &self.latest {
            Some(frame) => StreamFrame::Frame(frame.clone() as Rc<dyn VideoFrame>),
            None => StreamFrame::Pending,
        })
    }
}

impl Drop for WindowStream {
    fn drop(&mut self) {
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
