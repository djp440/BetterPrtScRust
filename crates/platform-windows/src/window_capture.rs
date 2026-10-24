//! Windows.Graphics.Capture（WGC）窗口捕获：把一个顶层窗口的内容持续送进帧池。
//!
//! WGC 截的是窗口自己的内容（DWM 的重定向表面），被其他窗口遮挡、超出屏幕的部分也在内，
//! 不含阴影。单张窗口截图（[`crate::WgcWindowSource`]）和窗口录屏共用 [`WindowCaptureSession`]：
//! 开始后系统在窗口内容变化时往帧池里放新帧，[`WindowCaptureSession::next_frame`] 取最新的一帧。
//!
//! - 帧池是 free-threaded 的：不需要 DispatcherQueue，帧到达的通知在系统线程池里发出。
//! - 每个会话自己建一个 D3D11 设备，用完随会话释放，不让常驻实例长期多占一份设备内存。
//! - Win11 上关闭黄色捕获边框（`IsBorderRequired = false`）；Win10 上系统一定会画，无法关闭。
//! - 不捕获鼠标指针（`IsCursorCaptureEnabled = false`，Win10 2004 起支持，更早的系统忽略）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use bps_platform::PlatformError;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIDevice3};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface, factory};

use crate::error::win_error;

/// 一次 WGC 窗口捕获会话。释放时停止捕获。
pub(crate) struct WindowCaptureSession {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// 持有捕获项，让它的 Closed 事件处理程序一直有效。
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// 帧池每放进一帧就收到一次通知。
    arrived: Receiver<()>,
    /// 窗口已关闭（或因其他原因不能再捕获）。
    closed: Arc<AtomicBool>,
}

/// 从帧池取出的一帧：BGRA8 纹理（尺寸是帧池尺寸）及其中有效内容的尺寸。
pub(crate) struct CapturedFrame {
    pub texture: ID3D11Texture2D,
    pub content_width: u32,
    pub content_height: u32,
    /// 帧要一直持有到不再使用纹理为止，释放后帧池才能复用这块缓冲区。
    _frame: Direct3D11CaptureFrame,
}

/// 读回 CPU 的一帧：BGRA8，alpha 预乘（DWM 的表面格式），行间无填充。
pub(crate) struct BgraFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl WindowCaptureSession {
    /// 开始捕获 `hwnd`。系统不支持 WGC（Win10 1903 以前）或窗口不能捕获时失败。
    pub fn start(hwnd: HWND) -> Result<Self, PlatformError> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(PlatformError::new(
                "系统不支持 Windows.Graphics.Capture（需要 Windows 10 1903 或更新版本）",
            ));
        }
        let (device, context) = create_device()?;
        unsafe {
            let dxgi: IDXGIDevice = device
                .cast()
                .map_err(|e| win_error("读取 DXGI 设备失败", e))?;
            let inspectable: IInspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .map_err(|e| win_error("创建 WinRT Direct3D 设备失败", e))?;
            let winrt_device: IDirect3DDevice = inspectable
                .cast()
                .map_err(|e| win_error("创建 WinRT Direct3D 设备失败", e))?;
            let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .map_err(|e| win_error("加载 Windows.Graphics.Capture 失败", e))?;
            let item: GraphicsCaptureItem = interop
                .CreateForWindow(hwnd)
                .map_err(|e| win_error("无法捕获这个窗口", e))?;
            let size = item.Size().map_err(|e| win_error("读取窗口尺寸失败", e))?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )
            .map_err(|e| win_error("创建捕获帧池失败", e))?;
            let session = pool
                .CreateCaptureSession(&item)
                .map_err(|e| win_error("创建捕获会话失败", e))?;
            // 两项都是较新系统才有的属性，不支持时保持系统默认。
            let _ = session.SetIsCursorCaptureEnabled(false);
            let _ = session.SetIsBorderRequired(false);

            let (sender, arrived) = channel();
            pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
                let _ = sender.send(());
                Ok(())
            }))
            .map_err(|e| win_error("订阅捕获帧失败", e))?;
            let closed = Arc::new(AtomicBool::new(false));
            let flag = closed.clone();
            item.Closed(&TypedEventHandler::new(move |_, _| {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }))
            .map_err(|e| win_error("订阅窗口关闭失败", e))?;
            session
                .StartCapture()
                .map_err(|e| win_error("开始捕获窗口失败", e))?;
            Ok(Self {
                device,
                context,
                _item: item,
                pool,
                session,
                arrived,
                closed,
            })
        }
    }

    /// 窗口已关闭，不会再有新帧。
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// 取帧池里最新的一帧（更早的帧丢弃）；没有新帧时最多等 `timeout`。
    /// 超时、窗口已关闭时返回错误。
    pub fn next_frame(&self, timeout: Duration) -> Result<CapturedFrame, PlatformError> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut latest = None;
            while let Ok(frame) = self.pool.TryGetNextFrame() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                return self.wrap(frame);
            }
            if self.is_closed() {
                return Err(PlatformError::new("窗口已关闭"));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self
                .arrived
                .recv_timeout(left.min(Duration::from_millis(50)))
            {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(_) => {
                    return Err(PlatformError::new(
                        "等待窗口画面超时（窗口可能已最小化、关闭或不允许捕获）",
                    ));
                }
            }
        }
    }

    fn wrap(&self, frame: Direct3D11CaptureFrame) -> Result<CapturedFrame, PlatformError> {
        let size = frame
            .ContentSize()
            .map_err(|e| win_error("读取捕获帧尺寸失败", e))?;
        let surface = frame
            .Surface()
            .map_err(|e| win_error("读取捕获帧表面失败", e))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|e| win_error("读取捕获帧表面失败", e))?;
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(|e| win_error("读取捕获帧纹理失败", e))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        // 窗口在捕获开始后变大时，内容尺寸可能超过帧池尺寸；只取纹理里有的部分。
        Ok(CapturedFrame {
            texture,
            content_width: (size.Width.max(0) as u32).min(desc.Width),
            content_height: (size.Height.max(0) as u32).min(desc.Height),
            _frame: frame,
        })
    }

    /// 把一帧的有效内容读回 CPU。
    pub fn read_back(&self, frame: &CapturedFrame) -> Result<BgraFrame, PlatformError> {
        let (width, height) = (frame.content_width, frame.content_height);
        if width == 0 || height == 0 {
            return Err(PlatformError::new("窗口画面是空的"));
        }
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut desc);
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..desc
            };
            let mut staging: Option<ID3D11Texture2D> = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|e| win_error("创建读回纹理失败", e))?;
            let staging = staging.ok_or_else(|| PlatformError::new("创建读回纹理失败"))?;
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: width,
                bottom: height,
                back: 1,
            };
            self.context.CopySubresourceRegion(
                &staging,
                0,
                0,
                0,
                0,
                &frame.texture,
                0,
                Some(&region),
            );
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| win_error("读回窗口画面失败", e))?;
            let row = width as usize * 4;
            let pitch = mapped.RowPitch as usize;
            let data = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                pitch * (height as usize - 1) + row,
            );
            let mut pixels = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                pixels.extend_from_slice(&data[y * pitch..y * pitch + row]);
            }
            self.context.Unmap(&staging, 0);
            Ok(BgraFrame {
                width,
                height,
                pixels,
            })
        }
    }
}

impl Drop for WindowCaptureSession {
    /// 停止捕获，并请驱动交还读回用的暂存内存（同 #8 遮罩的做法，否则会一直计入私有工作集）。
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
        unsafe {
            self.context.ClearState();
            self.context.Flush();
            if let Ok(device) = self.device.cast::<IDXGIDevice3>() {
                device.Trim();
            }
        }
    }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext), PlatformError> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| win_error("创建 D3D11 设备失败", e))?;
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(PlatformError::new("创建 D3D11 设备失败：未返回设备")),
    }
}
