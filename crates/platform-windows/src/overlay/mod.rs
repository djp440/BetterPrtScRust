//! 选区遮罩的 Windows 实现：原生 Win32 + D3D11 + Direct2D（ADR 0005）。
//!
//! 按 #2 原型的结论常驻预热：启动时就建好 D3D11 / Direct2D 设备、DirectWrite 和铺满虚拟桌面的
//! 隐藏遮罩窗口，并真实拍一次冻结帧、画两帧（含文字）把驱动和字体缓存热起来；
//! 之后每次打开遮罩只新建 Desktop Duplication 拍冻结帧（每块显示器 < 2 ms）。
//! 显示器布局变化（DXGI 工厂失效或收到 `WM_DISPLAYCHANGE`）后，下一次打开时整体重建。
//!
//! 代价：交换链常驻显存（4K 双缓冲约 66 MB，不计入进程内存）。

mod draw;
mod gpu;
mod window;

use bps_platform::{
    Backdrop, Color, Corner, Display, DisplayId, Image, Label, OverlayItem, OverlayScene,
    PlatformError, Point, Rect, SelectionOverlay,
};

use crate::event_loop::EventQueue;
use gpu::{FrozenDisplay, Gpu};
use window::OverlayWindow;

/// 选区遮罩。由 [`crate::EventLoop::selection_overlay`] 创建，必须在事件循环的线程上使用。
pub struct D2dSelectionOverlay {
    queue: EventQueue,
    resources: Option<Resources>,
    /// 资源出过错（例如显卡被移除），下次打开时重建。
    broken: bool,
    session: Option<Session>,
}

/// 常驻的设备和窗口。
struct Resources {
    gpu: Gpu,
    window: OverlayWindow,
}

/// 一次打开期间的状态。
struct Session {
    frozen: Vec<FrozenDisplay>,
    /// 已经显示出来了（第一次 `render` 之后）。
    shown: bool,
}

impl Resources {
    fn create(queue: &EventQueue) -> Result<Self, PlatformError> {
        let gpu = Gpu::new()?;
        let window = OverlayWindow::new(&gpu, gpu.virtual_bounds(), queue.clone())?;
        Ok(Self { gpu, window })
    }

    /// 画一帧并呈现。
    fn draw(&self, frozen: &[FrozenDisplay], scene: &OverlayScene) -> Result<(), PlatformError> {
        let bounds = self.window.bounds();
        let origin = Point::new(bounds.x, bounds.y);
        draw::draw(&self.gpu, &self.window.target, origin, frozen, scene)?;
        self.window.present()
    }
}

impl D2dSelectionOverlay {
    pub(crate) fn new(queue: EventQueue) -> Self {
        let mut overlay = Self {
            queue,
            resources: None,
            broken: false,
            session: None,
        };
        if let Err(e) = overlay.prewarm() {
            eprintln!("预热选区遮罩失败，将在第一次打开时重试：{e}");
        }
        overlay
    }

    /// 建好设备和窗口，真实拍一次冻结帧并在隐藏的窗口上画两帧（第二帧含文字）。
    fn prewarm(&mut self) -> Result<(), PlatformError> {
        let resources = Resources::create(&self.queue)?;
        let frozen = resources.gpu.freeze()?;
        let warm_up = warm_up_scene(&frozen);
        let drawn = resources
            .draw(&frozen, &OverlayScene::default())
            .and_then(|()| resources.draw(&frozen, &warm_up));
        drop(frozen);
        resources.gpu.release_frames();
        drawn?;
        self.resources = Some(resources);
        Ok(())
    }

    /// 确保设备和窗口可用且与当前显示器布局一致，必要时重建。
    fn ensure_resources(&mut self) -> Result<&Resources, PlatformError> {
        let usable = !self.broken
            && self
                .resources
                .as_ref()
                .is_some_and(|r| r.gpu.is_current() && !r.window.is_stale());
        if !usable {
            self.resources = None;
            self.resources = Some(Resources::create(&self.queue)?);
            self.broken = false;
        }
        Ok(self.resources.as_ref().expect("刚刚确保过"))
    }

    fn freeze(&mut self) -> Result<Vec<FrozenDisplay>, PlatformError> {
        let resources = self.ensure_resources()?;
        let frozen = resources.gpu.freeze()?;
        if covers(&frozen) == Some(resources.window.bounds()) {
            return Ok(frozen);
        }
        // 显示器位置变了但还没收到通知：重建一次。
        drop(frozen);
        self.broken = true;
        let resources = self.ensure_resources()?;
        resources.gpu.freeze()
    }
}

impl SelectionOverlay for D2dSelectionOverlay {
    fn open(&mut self, backdrop: Backdrop) -> Result<Vec<Display>, PlatformError> {
        let Backdrop::FrozenFrame = backdrop;
        if self.session.is_some() {
            return Err(PlatformError::new("选区遮罩已经打开"));
        }
        let frozen = self.freeze().inspect_err(|_| self.broken = true)?;
        let displays = frozen.iter().map(|f| f.display.clone()).collect();
        self.session = Some(Session {
            frozen,
            shown: false,
        });
        Ok(displays)
    }

    fn render(&mut self, scene: &OverlayScene) -> Result<(), PlatformError> {
        let (Some(resources), Some(session)) = (self.resources.as_ref(), self.session.as_mut())
        else {
            return Err(PlatformError::new("选区遮罩没有打开"));
        };
        let window = &resources.window;
        window.set_cursor(scene.cursor);
        let result = (|| {
            if !session.shown {
                window.place_cloaked()?;
            }
            resources.draw(&session.frozen, scene)
        })();
        if let Err(e) = result {
            self.broken = true;
            return Err(e);
        }
        if !session.shown {
            window.reveal();
            session.shown = true;
        }
        Ok(())
    }

    fn frozen_image(&self, display: DisplayId) -> Result<Image, PlatformError> {
        let (Some(resources), Some(session)) = (self.resources.as_ref(), self.session.as_ref())
        else {
            return Err(PlatformError::new("选区遮罩没有打开"));
        };
        let frozen = session
            .frozen
            .iter()
            .find(|f| f.display.id == display)
            .ok_or_else(|| PlatformError::new(format!("冻结帧中没有显示器 {display:?}")))?;
        resources.gpu.read_back(frozen)
    }

    fn close(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if let Some(resources) = &self.resources {
            resources.window.hide();
            drop(session);
            resources.gpu.release_frames();
        }
        // 关闭前已经排进队列的输入（如拖动中的鼠标移动）不再递交。
        self.queue.discard_overlay_input();
    }
}

/// 冻结帧覆盖的范围（各显示器的外接矩形）。
fn covers(frozen: &[FrozenDisplay]) -> Option<Rect> {
    frozen
        .iter()
        .map(|f| f.display.bounds)
        .reduce(|a, b| a.union(b))
}

/// 预热用的一帧：遮挡、边框和中英文标签，让字体回退和字形缓存提前加载。
fn warm_up_scene(frozen: &[FrozenDisplay]) -> OverlayScene {
    let Some(first) = frozen.first() else {
        return OverlayScene::default();
    };
    let b = first.display.bounds;
    let scale = first.display.scale_factor as f32;
    let region = Rect::new(b.x + 100, b.y + 100, 200, 100);
    OverlayScene {
        items: vec![
            OverlayItem::Fill {
                rect: b,
                color: Color::rgba(0, 0, 0, 115),
            },
            OverlayItem::Frame {
                rect: region,
                width: 1,
                color: Color::rgba(30, 144, 255, 255),
            },
            OverlayItem::Label(Label {
                text: "200 × 100　Enter 或双击选区：完成　Esc：取消".to_owned(),
                anchor: Point::new(region.x, region.y),
                corner: Corner::BottomLeft,
                font_size: 13.0 * scale,
                padding: 4,
                color: Color::rgba(255, 255, 255, 255),
                background: Color::rgba(0, 0, 0, 180),
            }),
        ],
        ..OverlayScene::default()
    }
}
