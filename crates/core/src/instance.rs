use bps_platform::{HotkeyId, IncomingRequest, OverlayInput, Platform, PlatformEvent};

use crate::binding::HotkeyRegistrationFailure;
use crate::region::{Ending, RegionCapture};
use crate::{
    CaptureMode, CaptureRequest, Command, CoreError, Flow, HotkeyBinding, PostCaptureAction, Reply,
    Request, action, capture,
};

/// 常驻实例的核心：持有平台能力和热键绑定，把触发事件转换成命令并执行。
///
/// 宿主（`app` 里的平台事件循环）把 [`PlatformEvent`] 交给 [`handle`](Self::handle)，
/// 在返回 [`Flow::Exit`] 时退出循环。
///
/// 需要用户在选区遮罩上操作的捕获（区域）不会在一次调用里完成：命令打开遮罩后就返回，
/// 之后的 [`PlatformEvent::OverlayInput`] 推进这次捕获，直到用户确定或取消。
/// 捕获进行期间，新的捕获命令会被拒绝（[`CoreError::CaptureInProgress`]）。
pub struct ResidentInstance {
    platform: Platform,
    bindings: Vec<HotkeyBinding>,
    /// 已成功注册的热键，下标与 `bindings` 对应。
    registered: Vec<HotkeyId>,
    /// 进行中的区域捕获。
    region_capture: Option<RegionCapture>,
}

/// 命令执行到哪一步了。
enum Progress {
    /// 命令已执行完。
    Done(Flow),
    /// 已开始一次需要用户操作的捕获，结束时才有结果。
    Pending,
}

impl ResidentInstance {
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            platform,
            bindings,
            registered: Vec::new(),
            region_capture: None,
        }
    }

    /// 为每条热键绑定注册全局热键，返回注册失败的绑定；失败的绑定不影响其他绑定。
    #[must_use = "热键注册失败不能静默忽略"]
    pub fn register_hotkeys(&mut self) -> Vec<HotkeyRegistrationFailure> {
        let mut failures = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            let id = hotkey_id(index);
            match self.platform.hotkeys.register(id, binding.hotkey) {
                Ok(()) => self.registered.push(id),
                Err(error) => failures.push(HotkeyRegistrationFailure {
                    binding: binding.clone(),
                    error,
                }),
            }
        }
        failures
    }

    /// 把平台事件转换成命令；与任何绑定都无关的事件、无法识别的请求、遮罩上的输入返回 `None`。
    pub fn command_for(&self, event: &PlatformEvent) -> Option<Command> {
        match event {
            PlatformEvent::HotkeyPressed(id) => self
                .bindings
                .iter()
                .enumerate()
                .find(|(index, _)| hotkey_id(*index) == *id)
                .map(|(_, binding)| Command::Capture(binding.capture.clone())),
            PlatformEvent::ExitRequested => Some(Command::Exit),
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).map(|r| self.command_for_request(&r))
            }
            PlatformEvent::OverlayInput(_) => None,
        }
    }

    /// 处理一个平台事件：转换成命令后执行；遮罩上的输入交给进行中的捕获。
    ///
    /// 请求事件在命令执行完后回复发起方；需要用户操作的捕获则等捕获结束再回复。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match event {
            PlatformEvent::RequestReceived(request) => self.serve(request),
            PlatformEvent::OverlayInput(input) => self.on_overlay_input(&input),
            event => match self.command_for(&event) {
                Some(command) => self.execute(command),
                None => Ok(Flow::Continue),
            },
        }
    }

    /// 执行转交来的请求，并把结果回复给发起方。
    fn serve(&mut self, incoming: IncomingRequest) -> Result<Flow, CoreError> {
        let result = match Request::decode(incoming.message()) {
            Some(request) => {
                let command = self.command_for_request(&request);
                self.run(command)
            }
            None => Err(CoreError::InvalidRequest(incoming.message().to_owned())),
        };
        match result {
            Ok(Progress::Done(flow)) => {
                incoming.reply(Reply::Done.encode());
                Ok(flow)
            }
            Ok(Progress::Pending) => {
                if let Some(capture) = self.region_capture.as_mut() {
                    capture.requester = Some(incoming);
                }
                Ok(Flow::Continue)
            }
            Err(e) => {
                incoming.reply(reply_for_error(&e).encode());
                Err(e)
            }
        }
    }

    /// 请求对应的命令。`capture <捕获模式>` 与按下该捕获模式的热键完全一致：
    /// 采用第一条该捕获模式的热键绑定；没有这样的绑定时只复制到剪贴板。
    fn command_for_request(&self, request: &Request) -> Command {
        match request {
            Request::Capture(mode) => Command::Capture(
                self.bindings
                    .iter()
                    .find(|binding| binding.capture.mode == *mode)
                    .map(|binding| binding.capture.clone())
                    .unwrap_or_else(|| {
                        CaptureRequest::new(*mode, vec![PostCaptureAction::CopyToClipboard])
                    }),
            ),
            Request::Activate => Command::Activate,
        }
    }

    /// 执行一条命令。
    ///
    /// 区域捕获在显示出选区遮罩后就返回 `Ok(Flow::Continue)`，之后由遮罩上的输入事件推进。
    pub fn execute(&mut self, command: Command) -> Result<Flow, CoreError> {
        match self.run(command)? {
            Progress::Done(flow) => Ok(flow),
            Progress::Pending => Ok(Flow::Continue),
        }
    }

    fn run(&mut self, command: Command) -> Result<Progress, CoreError> {
        match command {
            Command::Capture(request) => self.capture(request),
            Command::Activate => Ok(Progress::Done(Flow::Continue)),
            Command::Exit => {
                self.abandon_region_capture();
                self.unregister_hotkeys();
                Ok(Progress::Done(Flow::Exit))
            }
        }
    }

    fn capture(&mut self, request: CaptureRequest) -> Result<Progress, CoreError> {
        if self.region_capture.is_some() {
            return Err(CoreError::CaptureInProgress);
        }
        if request.mode == CaptureMode::Region {
            self.region_capture = Some(RegionCapture::start(&mut self.platform, request)?);
            return Ok(Progress::Pending);
        }
        let screenshot = capture::capture(&self.platform, request.mode)?;
        action::run(&mut self.platform, &screenshot, &request)?;
        Ok(Progress::Done(Flow::Continue))
    }

    /// 把遮罩上的输入交给进行中的区域捕获；捕获结束时回复等待中的请求。
    fn on_overlay_input(&mut self, input: &OverlayInput) -> Result<Flow, CoreError> {
        let Some(capture) = self.region_capture.as_mut() else {
            return Ok(Flow::Continue);
        };
        let Some(ending) = capture.handle(&mut self.platform, input) else {
            return Ok(Flow::Continue);
        };
        let requester = self
            .region_capture
            .take()
            .and_then(|capture| capture.requester);
        if let Some(requester) = requester {
            let reply = match &ending {
                Ok(Ending::Captured) => Reply::Done,
                Ok(Ending::Cancelled) => Reply::Cancelled,
                Err(e) => reply_for_error(e),
            };
            requester.reply(reply.encode());
        }
        ending.map(|_| Flow::Continue)
    }

    /// 放弃进行中的区域捕获：关闭遮罩，等待中的请求收到“已取消”。
    fn abandon_region_capture(&mut self) {
        if let Some(capture) = self.region_capture.take() {
            self.platform.overlay.close();
            if let Some(requester) = capture.requester {
                requester.reply(Reply::Cancelled.encode());
            }
        }
    }

    /// 注销所有已注册的热键。尽力而为：某个热键注销失败不影响其他热键，
    /// 进程退出时系统也会回收它们。
    fn unregister_hotkeys(&mut self) {
        for id in self.registered.drain(..) {
            let _ = self.platform.hotkeys.unregister(id);
        }
    }
}

/// 命令执行失败时给请求发起方的回复。
fn reply_for_error(error: &CoreError) -> Reply {
    match error {
        CoreError::UnsupportedCaptureMode(_) => Reply::Unsupported(error.to_string()),
        _ => Reply::Failed(error.to_string()),
    }
}

/// 第 `index` 条绑定使用的热键标识。
fn hotkey_id(index: usize) -> HotkeyId {
    HotkeyId(index as u32 + 1)
}
