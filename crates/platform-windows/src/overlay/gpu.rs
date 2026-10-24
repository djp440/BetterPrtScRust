//! 选区遮罩常驻的 GPU 资源与冻结帧。
//!
//! - 每块接着桌面显示器的显卡一个 D3D11 设备：Desktop Duplication 只能用显示器所在显卡的设备。
//! - 渲染显卡（主显示器所在的显卡）上的 Direct2D 设备上下文和 DirectWrite，DPI 固定 96，
//!   所以 D2D 的 DIP 就是物理像素。
//!
//! 冻结帧每块显示器一张：SDR 桌面、在渲染显卡上、没有旋转时，复制成 GPU 纹理后直接包成
//! D2D 位图（不经 CPU，延迟最低）；HDR（FP16）、跨显卡或旋转的显示器先读回 CPU，按 #3 的
//! 转换得到 RGBA，再上传成位图。

use std::cell::RefCell;

use bps_platform::{Display, Image, PlatformError, Rect};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_U, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1CreateFactory,
    ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_OUTPUT_DESC, IDXGIAdapter1, IDXGIDevice,
    IDXGIDevice3, IDXGIFactory1, IDXGIOutput, IDXGISurface,
};
use windows::core::{Interface, w};

use crate::error::win_error;
use crate::frame_source::{acquire_frame, copy_to_cpu, describe_monitor, duplicate_output};
use crate::pixels;

/// 一块显卡及其上的 D3D11 设备。
struct Adapter {
    adapter: IDXGIAdapter1,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

/// 一块接着桌面的显示器。
struct Output {
    output: IDXGIOutput,
    desc: DXGI_OUTPUT_DESC,
    /// 在 `Gpu::adapters` 中的下标。
    adapter: usize,
}

/// 冻结帧中的一块显示器。
pub(crate) struct FrozenDisplay {
    pub display: Display,
    /// 画在遮罩上的位图（渲染显卡上）。
    pub bitmap: ID2D1Bitmap1,
    pixels: FrozenPixels,
    output_desc: DXGI_OUTPUT_DESC,
}

enum FrozenPixels {
    /// 渲染显卡上的 BGRA8 纹理，需要时再读回。
    Texture(ID3D11Texture2D),
    /// 已转换好的 RGBA。
    Image(Image),
}

pub(crate) struct Gpu {
    factory: IDXGIFactory1,
    adapters: Vec<Adapter>,
    outputs: Vec<Output>,
    /// 渲染用的显卡在 `adapters` 中的下标。
    render: usize,
    pub d2d: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    pub brush: ID2D1SolidColorBrush,
    /// 每个字号、对齐方式一个文字格式（字号按 1/100 像素取整，与是否居中一起作键）。
    text_formats: RefCell<Vec<((u32, bool), IDWriteTextFormat)>>,
}

impl Gpu {
    pub fn new() -> Result<Self, PlatformError> {
        unsafe {
            let factory: IDXGIFactory1 =
                CreateDXGIFactory1().map_err(|e| win_error("创建 DXGI 工厂失败", e))?;
            let mut adapters = Vec::new();
            let mut outputs = Vec::new();
            for adapter_index in 0.. {
                let adapter = match factory.EnumAdapters1(adapter_index) {
                    Ok(adapter) => adapter,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(win_error("枚举显卡失败", e)),
                };
                let mut found = Vec::new();
                for output_index in 0.. {
                    let output = match adapter.EnumOutputs(output_index) {
                        Ok(output) => output,
                        Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                        Err(e) => return Err(win_error("枚举显示输出失败", e)),
                    };
                    let desc = output
                        .GetDesc()
                        .map_err(|e| win_error("读取显示输出失败", e))?;
                    if desc.AttachedToDesktop.as_bool() {
                        found.push((output, desc));
                    }
                }
                if found.is_empty() {
                    continue;
                }
                let (device, context) = create_device(&adapter)?;
                let index = adapters.len();
                adapters.push(Adapter {
                    adapter,
                    device,
                    context,
                });
                outputs.extend(found.into_iter().map(|(output, desc)| Output {
                    output,
                    desc,
                    adapter: index,
                }));
            }
            if outputs.is_empty() {
                return Err(PlatformError::new("没有接入桌面的显示器"));
            }
            let render = outputs
                .iter()
                .find(|o| describe_monitor(o.desc.Monitor).is_ok_and(|d| d.is_primary))
                .map_or(0, |o| o.adapter);

            let dxgi_device: IDXGIDevice = adapters[render]
                .device
                .cast()
                .map_err(|e| win_error("读取 DXGI 设备失败", e))?;
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                    .map_err(|e| win_error("创建 Direct2D 工厂失败", e))?;
            let d2d = d2d_factory
                .CreateDevice(&dxgi_device)
                .and_then(|device| device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE))
                .map_err(|e| win_error("创建 Direct2D 设备失败", e))?;
            // DIP = 物理像素；缩放由 core 按显示器换算好。
            d2d.SetDpi(96.0, 96.0);
            let brush = d2d
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    None,
                )
                .map_err(|e| win_error("创建画刷失败", e))?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
                .map_err(|e| win_error("创建 DirectWrite 工厂失败", e))?;
            Ok(Self {
                factory,
                adapters,
                outputs,
                render,
                d2d,
                dwrite,
                brush,
                text_formats: RefCell::new(Vec::new()),
            })
        }
    }

    /// 显卡或显示器拓扑变化后 DXGI 工厂会失效，需要整体重建。
    pub fn is_current(&self) -> bool {
        unsafe { self.factory.IsCurrent().as_bool() }
    }

    pub fn render_device(&self) -> &ID3D11Device {
        &self.adapters[self.render].device
    }

    pub fn render_adapter(&self) -> &IDXGIAdapter1 {
        &self.adapters[self.render].adapter
    }

    /// 所有显示器的外接矩形。
    pub fn virtual_bounds(&self) -> Rect {
        self.outputs
            .iter()
            .map(|o| rect_of(&o.desc))
            .reduce(|a, b| a.union(b))
            .unwrap_or_default()
    }

    pub fn dwrite(&self) -> &IDWriteFactory {
        &self.dwrite
    }

    /// 拍冻结帧：每块显示器各新建一次 Desktop Duplication（很快），拍完即释放。
    pub fn freeze(&self) -> Result<Vec<FrozenDisplay>, PlatformError> {
        self.outputs
            .iter()
            .map(|output| unsafe { self.freeze_output(output) })
            .collect()
    }

    unsafe fn freeze_output(&self, output: &Output) -> Result<FrozenDisplay, PlatformError> {
        let display = describe_monitor(output.desc.Monitor)?;
        let adapter = &self.adapters[output.adapter];
        unsafe {
            let duplication = duplicate_output(&output.output, &adapter.device)?;
            let rotation = duplication.GetDesc().Rotation;
            let (texture, frame) = acquire_frame(&duplication)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let unrotated = rotation == DXGI_MODE_ROTATION_IDENTITY
                || rotation == DXGI_MODE_ROTATION_UNSPECIFIED;
            if desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM
                && output.adapter == self.render
                && unrotated
            {
                let copy_desc = D3D11_TEXTURE2D_DESC {
                    MipLevels: 1,
                    ArraySize: 1,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                    ..desc
                };
                let mut copy: Option<ID3D11Texture2D> = None;
                adapter
                    .device
                    .CreateTexture2D(&copy_desc, None, Some(&mut copy))
                    .map_err(|e| win_error("创建冻结帧纹理失败", e))?;
                let copy = copy.ok_or_else(|| PlatformError::new("创建冻结帧纹理失败"))?;
                adapter.context.CopyResource(&copy, &texture);
                drop(frame);
                let surface: IDXGISurface = copy
                    .cast()
                    .map_err(|e| win_error("读取冻结帧表面失败", e))?;
                let bitmap = self
                    .d2d
                    .CreateBitmapFromDxgiSurface(
                        &surface,
                        Some(&bitmap_properties(
                            DXGI_FORMAT_B8G8R8A8_UNORM,
                            D2D1_BITMAP_OPTIONS_NONE,
                        )),
                    )
                    .map_err(|e| win_error("创建冻结帧位图失败", e))?;
                Ok(FrozenDisplay {
                    display,
                    bitmap,
                    pixels: FrozenPixels::Texture(copy),
                    output_desc: output.desc,
                })
            } else {
                let image = copy_to_cpu(&adapter.device, &adapter.context, &texture, &output.desc)?;
                drop(frame);
                let image = pixels::rotate(image, rotation);
                let bitmap = self
                    .d2d
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: image.width(),
                            height: image.height(),
                        },
                        Some(image.rgba().as_ptr().cast()),
                        image.width() * 4,
                        &bitmap_properties(DXGI_FORMAT_R8G8B8A8_UNORM, D2D1_BITMAP_OPTIONS_NONE),
                    )
                    .map_err(|e| win_error("创建冻结帧位图失败", e))?;
                Ok(FrozenDisplay {
                    display,
                    bitmap,
                    pixels: FrozenPixels::Image(image),
                    output_desc: output.desc,
                })
            }
        }
    }

    /// 用 RGBA 图像新建画布位图（渲染显卡上，忽略 alpha）。
    pub fn create_canvas(&self, image: &Image) -> Result<ID2D1Bitmap1, PlatformError> {
        unsafe {
            self.d2d
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: image.width(),
                        height: image.height(),
                    },
                    Some(image.rgba().as_ptr().cast()),
                    image.width() * 4,
                    &bitmap_properties(DXGI_FORMAT_R8G8B8A8_UNORM, D2D1_BITMAP_OPTIONS_NONE),
                )
                .map_err(|e| win_error("创建画布位图失败", e))
        }
    }

    /// 把 `patch` 复制到画布位图上以 `(left, top)`（位图内像素坐标）为左上角的位置。
    pub fn update_canvas(
        &self,
        canvas: &ID2D1Bitmap1,
        left: u32,
        top: u32,
        patch: &Image,
    ) -> Result<(), PlatformError> {
        if patch.width() == 0 || patch.height() == 0 {
            return Ok(());
        }
        let rect = D2D_RECT_U {
            left,
            top,
            right: left + patch.width(),
            bottom: top + patch.height(),
        };
        unsafe {
            canvas
                .CopyFromMemory(Some(&rect), patch.rgba().as_ptr().cast(), patch.width() * 4)
                .map_err(|e| win_error("更新画布失败", e))
        }
    }

    /// 冻结帧中这块显示器的画面（RGBA）。只读取已拍下的冻结帧，不重新截图。
    pub fn read_back(&self, frozen: &FrozenDisplay) -> Result<Image, PlatformError> {
        match &frozen.pixels {
            FrozenPixels::Image(image) => Ok(image.clone()),
            FrozenPixels::Texture(texture) => unsafe {
                let render = &self.adapters[self.render];
                copy_to_cpu(
                    &render.device,
                    &render.context,
                    texture,
                    &frozen.output_desc,
                )
            },
        }
    }

    /// 让 D3D11 立即回收已释放的冻结帧纹理，并请驱动交还为读回分配的内存。
    ///
    /// 读回用的暂存纹理释放后，驱动会把那块内存（4K HDR 约 63 MB）留在进程里备用，
    /// 计入私有工作集；`IDXGIDevice3::Trim` 让它交还（约 8 ms，在遮罩隐藏之后做）。
    /// 代价是下一次读回要重新分配，实测对热键 → 遮罩延迟没有可见影响。
    pub fn release_frames(&self) {
        for adapter in &self.adapters {
            unsafe {
                adapter.context.ClearState();
                adapter.context.Flush();
                if let Ok(device) = adapter.device.cast::<IDXGIDevice3>() {
                    device.Trim();
                }
            }
        }
    }

    /// 字号为 `font_size`（物理像素）的文字格式：微软雅黑 UI 半粗体、不换行、左上对齐。
    pub fn text_format(&self, font_size: f32) -> Result<IDWriteTextFormat, PlatformError> {
        self.cached_text_format(font_size, false)
    }

    /// 同 [`text_format`](Self::text_format)，但在排版矩形内水平、垂直都居中。
    pub fn centered_text_format(&self, font_size: f32) -> Result<IDWriteTextFormat, PlatformError> {
        self.cached_text_format(font_size, true)
    }

    fn cached_text_format(
        &self,
        font_size: f32,
        centered: bool,
    ) -> Result<IDWriteTextFormat, PlatformError> {
        let key = ((font_size * 100.0).round() as u32, centered);
        let mut formats = self.text_formats.borrow_mut();
        if let Some((_, format)) = formats.iter().find(|(k, _)| *k == key) {
            return Ok(format.clone());
        }
        let format = unsafe {
            let format = self
                .dwrite
                .CreateTextFormat(
                    w!("Microsoft YaHei UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    w!("zh-CN"),
                )
                .map_err(|e| win_error("创建文字格式失败", e))?;
            format
                .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                .map_err(|e| win_error("设置文字格式失败", e))?;
            if centered {
                format
                    .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)
                    .and_then(|()| format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER))
                    .map_err(|e| win_error("设置文字格式失败", e))?;
            }
            format
        };
        formats.push((key, format.clone()));
        Ok(format)
    }
}

/// 显示器在虚拟桌面中的范围。
fn rect_of(desc: &DXGI_OUTPUT_DESC) -> Rect {
    let r = desc.DesktopCoordinates;
    Rect::new(
        r.left,
        r.top,
        (r.right - r.left) as u32,
        (r.bottom - r.top) as u32,
    )
}

/// 忽略 alpha 的位图属性：桌面复制出来的 alpha 不可靠。
pub(crate) fn bitmap_properties(
    format: DXGI_FORMAT,
    options: D2D1_BITMAP_OPTIONS,
) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        ..Default::default()
    }
}

fn create_device(
    adapter: &IDXGIAdapter1,
) -> Result<(ID3D11Device, ID3D11DeviceContext), PlatformError> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ]),
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
