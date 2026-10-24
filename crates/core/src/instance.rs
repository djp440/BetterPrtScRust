use bps_platform::{
    AutostartMethod, Hotkey, HotkeyId, IncomingRequest, Key, Modifiers, Notification,
    NotificationClick, OverlayInput, Platform, PlatformError, PlatformEvent,
};

use crate::capture::CaptureFailure;
use crate::recording::{self, Recording};
use crate::region::{Ending, RegionCapture};
use crate::{
    CaptureMode, CaptureRequest, Command, Config, CoreError, DEFAULT_CONFIG, Flow, HotkeyBinding,
    HotkeyRegistrationFailure, PostCaptureAction, RegistrationError, Reply, Request, action,
    capture,
};

/// 宿主启动常驻实例时提供的运行环境。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartOptions {
    /// 以便携模式运行：开启开机自启时提醒用户，移动程序目录后需要重新开启。
    pub portable: bool,
    /// 本进程是为了以管理员身份运行而重新启动的：如果仍然没有管理员权限，不再重试。
    pub restarted_as_administrator: bool,
}

/// 常驻实例的核心：持有平台能力和配置，把触发事件转换成命令并执行。
///
/// 宿主（`app` 里的平台事件循环）用 [`start_with`](Self::start_with) 创建它，把
/// [`PlatformEvent`] 交给 [`handle`](Self::handle)，在返回 [`Flow::Exit`] 时退出循环。
///
/// 需要用户在选区遮罩上操作的捕获（区域）不会在一次调用里完成：命令打开遮罩后就返回，
/// 之后的 [`PlatformEvent::OverlayInput`] 推进这次捕获，直到用户确定或取消。
/// 捕获进行期间，新的捕获命令会被拒绝（[`CoreError::CaptureInProgress`]）。
///
/// 录屏同样跨越很多事件：框选阶段同上；录制期间由 [`PlatformEvent::Tick`] 推进，
/// 再次执行同一个捕获请求（再按一次同一个热键）时停止并存储，其他捕获命令被拒绝
/// （[`CoreError::RecordingInProgress`]）。
pub struct ResidentInstance {
    platform: Platform,
    config: Config,
    options: StartOptions,
    /// 已成功注册的热键。
    registered: Vec<HotkeyId>,
    /// 最近一次注册热键时失败的绑定。
    failures: Vec<HotkeyRegistrationFailure>,
    /// 已以管理员身份启动了接替本实例的新实例。
    restarting_as_administrator: bool,
    /// 进行中的区域捕获。
    region_capture: Option<RegionCapture>,
    /// 进行中的录屏（框选区域或正在录制）。
    recording: Option<Recording>,
}

/// 命令执行到哪一步了。
enum Progress {
    /// 命令已执行完。
    Done(Flow),
    /// 已开始一次需要用户操作的捕获，结束时才有结果。
    Pending,
}

impl ResidentInstance {
    /// 用给定的热键绑定创建常驻实例，不读写配置文件、不注册热键、不动开机自启。
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            platform,
            config: Config::with_bindings(bindings),
            options: StartOptions::default(),
            registered: Vec::new(),
            failures: Vec::new(),
            restarting_as_administrator: false,
            region_capture: None,
            recording: None,
        }
    }

    /// 以默认的运行环境（非便携模式）启动常驻实例，见 [`start_with`](Self::start_with)。
    pub fn start(platform: Platform) -> Self {
        Self::start_with(platform, StartOptions::default())
    }

    /// 启动常驻实例：读取配置文件（不存在时写入默认配置）；配置要求以管理员身份运行而
    /// 本进程没有管理员权限时，以管理员身份重新启动（见
    /// [`restarting_as_administrator`](Self::restarting_as_administrator)）；否则注册热键，
    /// 按配置登记或移除开机自启，并开始监视配置文件和 PrtSc 键的系统设置。
    ///
    /// 不会失败：配置文件有错误、热键注册失败等问题都通过通知告诉用户，
    /// 有问题的部分用默认值代替或跳过。
    pub fn start_with(platform: Platform, options: StartOptions) -> Self {
        let mut instance = Self::new(platform, Vec::new());
        instance.options = options;
        instance.config = instance.load_config_at_startup();
        if instance.config.run_as_admin && !instance.platform.elevation.is_elevated() {
            if options.restarted_as_administrator {
                instance.notify_not_elevated("以管理员身份重新启动后仍然没有管理员权限。");
            } else if instance.restart_as_administrator() {
                return instance;
            }
        }
        let _ = instance.register_hotkeys();
        instance.apply_autostart();
        if let Err(e) = instance.platform.config_file.watch() {
            instance.notify_with_config_link(
                "无法监视配置文件",
                format!("修改配置文件后需要重启 BetterPrtSc 才能生效。{e}"),
            );
        }
        // 监视失败只是少了“关闭系统设置后立即接管 PrtSc”，重启后照样生效，不打扰用户。
        let _ = instance.platform.system_screenshot_tool.watch();
        instance
    }

    /// 本实例已以管理员身份启动了接替它的新实例：宿主应立即退出、让出单实例，
    /// 不再进入事件循环。此时本实例没有注册任何热键。
    pub fn restarting_as_administrator(&self) -> bool {
        self.restarting_as_administrator
    }

    /// 当前生效的配置。
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// 最近一次注册热键时失败的绑定，供设置界面把它们标红。
    pub fn registration_failures(&self) -> &[HotkeyRegistrationFailure] {
        &self.failures
    }

    /// 按当前配置（重新）注册全部热键，返回注册失败的绑定。
    ///
    /// 失败的绑定不影响其他绑定；有失败时发出通知，并可用
    /// [`registration_failures`](Self::registration_failures) 查询。PrtSc 被系统截图工具
    /// 占用时另发一条引导通知，点击打开对应的系统设置页面。
    pub fn register_hotkeys(&mut self) -> &[HotkeyRegistrationFailure] {
        self.unregister_hotkeys();
        self.failures.clear();
        for index in 0..self.config.bindings.len() {
            let hotkey = self.config.bindings[index].hotkey;
            let error = match self.config.bindings[..index]
                .iter()
                .position(|earlier| earlier.hotkey == hotkey)
            {
                Some(first) => RegistrationError::DuplicateOf(first),
                None => match self.try_register(index) {
                    Ok(()) => continue,
                    Err(error) => error,
                },
            };
            self.failures.push(HotkeyRegistrationFailure {
                index,
                binding: self.config.bindings[index].clone(),
                error,
            });
        }
        self.notify_registration_failures(&self.failures);
        &self.failures
    }

    /// 注册第 `index` 条绑定的热键。不带修饰键的 PrtSc 注册失败时，查看系统设置是否把
    /// PrtSc 键交给了系统截图工具（只读取，不修改）。
    fn try_register(&mut self, index: usize) -> Result<(), RegistrationError> {
        let hotkey = self.config.bindings[index].hotkey;
        match self.platform.hotkeys.register(hotkey_id(index), hotkey) {
            Ok(()) => {
                self.registered.push(hotkey_id(index));
                Ok(())
            }
            Err(_)
                if hotkey == PRINT_SCREEN
                    && self
                        .platform
                        .system_screenshot_tool
                        .takes_print_screen_key()
                        .unwrap_or(false) =>
            {
                Err(RegistrationError::TakenBySystemScreenshotTool)
            }
            Err(e) => Err(RegistrationError::Platform(e)),
        }
    }

    /// 为注册失败的绑定发通知：系统截图工具占用 PrtSc 的发引导通知，其余的汇总成一条。
    fn notify_registration_failures(&self, failures: &[HotkeyRegistrationFailure]) {
        let (taken_by_system, others): (Vec<_>, Vec<_>) = failures
            .iter()
            .partition(|f| f.error == RegistrationError::TakenBySystemScreenshotTool);
        if !others.is_empty() {
            let body = others
                .iter()
                .map(|f| format!("{}：{}", f.binding.hotkey, f.error))
                .collect::<Vec<_>>()
                .join("\n");
            self.notify_with_config_link("热键注册失败", body);
        }
        if !taken_by_system.is_empty() {
            self.platform.notifier.notify(Notification {
                title: "PrtSc 键被系统截图工具占用".to_owned(),
                body: "点击打开系统设置，关闭“使用 Print Screen 键打开屏幕截图”后，\
                       BetterPrtSc 会立即接管 PrtSc 键，无需重启。"
                    .to_owned(),
                on_click: Some(NotificationClick::OpenPrintScreenKeySettings),
            });
        }
    }

    /// PrtSc 键的系统设置变化后：如果系统截图工具不再占用 PrtSc，重新注册之前因此失败的绑定。
    fn retry_print_screen_bindings(&mut self) {
        let waiting: Vec<usize> = self
            .failures
            .iter()
            .filter(|f| f.error == RegistrationError::TakenBySystemScreenshotTool)
            .map(|f| f.index)
            .collect();
        if waiting.is_empty()
            || self
                .platform
                .system_screenshot_tool
                .takes_print_screen_key()
                .unwrap_or(true)
        {
            return;
        }
        self.failures
            .retain(|f| f.error != RegistrationError::TakenBySystemScreenshotTool);
        let mut still_failing = Vec::new();
        for index in waiting {
            if let Err(error) = self.try_register(index) {
                still_failing.push(HotkeyRegistrationFailure {
                    index,
                    binding: self.config.bindings[index].clone(),
                    error,
                });
            }
        }
        if still_failing.is_empty() {
            self.platform.notifier.notify(Notification {
                title: "PrtSc 键已可用".to_owned(),
                body: "系统截图工具不再占用 PrtSc 键，相应的热键绑定已生效。".to_owned(),
                on_click: None,
            });
        }
        self.notify_registration_failures(&still_failing);
        self.failures.extend(still_failing);
        self.failures.sort_by_key(|f| f.index);
    }

    /// 把平台事件转换成命令；与任何绑定都无关的事件、无法识别的请求、遮罩上的输入返回 `None`。
    pub fn command_for(&self, event: &PlatformEvent) -> Option<Command> {
        match event {
            PlatformEvent::HotkeyPressed(id) => self
                .config
                .bindings
                .iter()
                .enumerate()
                .find(|(index, _)| hotkey_id(*index) == *id)
                .map(|(_, binding)| Command::Capture(binding.capture.clone())),
            PlatformEvent::ExitRequested => Some(Command::Exit),
            PlatformEvent::ConfigFileChanged | PlatformEvent::PrintScreenKeySettingChanged => None,
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).map(|r| self.command_for_request(&r))
            }
            PlatformEvent::OverlayInput(_) | PlatformEvent::Tick => None,
        }
    }

    /// 处理一个平台事件：配置文件变更时重新加载配置；PrtSc 键的系统设置变化时重试被系统
    /// 截图工具挡住的热键绑定；遮罩上的输入交给进行中的区域捕获或录屏，节拍交给进行中的录屏；请求事件执行后把结果
    /// 回复给发起方（需要用户操作的捕获等捕获结束再回复）；其余事件转换成命令后执行，
    /// 失败时发通知。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match event {
            PlatformEvent::ConfigFileChanged => return Ok(self.reload_config()),
            PlatformEvent::PrintScreenKeySettingChanged => {
                self.retry_print_screen_bindings();
                return Ok(Flow::Continue);
            }
            PlatformEvent::RequestReceived(request) => return self.serve(request),
            PlatformEvent::OverlayInput(input) => return self.on_overlay_input(&input),
            PlatformEvent::Tick => return self.advance_recording(Recording::on_tick),
            PlatformEvent::HotkeyPressed(_) | PlatformEvent::ExitRequested => {}
        }
        let Some(command) = self.command_for(&event) else {
            return Ok(Flow::Continue);
        };
        let Command::Capture(request) = command else {
            return self.execute(command);
        };
        // 热键触发的捕获没有别的反馈渠道，失败时用通知告诉用户（截后动作已经通知过的除外）。
        match self.capture(&request) {
            Ok(_) => Ok(Flow::Continue),
            Err(failure) => {
                self.notify_capture_failure(&failure);
                Err(failure.error)
            }
        }
    }

    /// 热键触发的捕获失败时通知用户；截后动作已经就这次失败通知过的不再重复。
    fn notify_capture_failure(&self, failure: &CaptureFailure) {
        if failure.notified {
            return;
        }
        let title = match failure.error {
            CoreError::UnsupportedCaptureMode(_)
            | CoreError::UnsupportedAction(_)
            | CoreError::UnsupportedRecordingFormat(_) => "暂不支持",
            CoreError::RecordingInProgress { .. } => "正在录屏",
            _ => "捕获失败",
        };
        self.platform.notifier.notify(Notification {
            title: title.to_owned(),
            body: failure.error.to_string(),
            on_click: None,
        });
    }

    /// 执行转交来的请求，并把结果回复给发起方；需要用户操作的捕获等捕获结束再回复。
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
                } else if let Some(recording) = self.recording.as_mut() {
                    recording.requester = Some(incoming);
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
                self.config
                    .bindings
                    .iter()
                    .find(|binding| binding.capture.mode == *mode)
                    .map(|binding| binding.capture.clone())
                    .unwrap_or_else(|| CaptureRequest {
                        output: self.config.output.clone(),
                        ..CaptureRequest::new(*mode, vec![PostCaptureAction::CopyToClipboard])
                    }),
            ),
            Request::Activate => Command::Activate,
        }
    }

    /// 执行一次捕获：先确认捕获模式和截后动作都已实现，再捕获、执行截后动作。
    ///
    /// 区域捕获只拍下冻结帧、显示选区遮罩（[`Progress::Pending`]），之后由遮罩上的输入推进。
    fn capture(&mut self, request: &CaptureRequest) -> Result<Progress, CaptureFailure> {
        if let Some(recording) = &self.recording {
            if recording.is_recording() && recording.request() == request {
                // 再按一次开始录屏的热键：停止录制。录屏的结果已经回复给发起录屏的请求或通知过用户。
                return match self.stop_recording() {
                    Ok(flow) => Ok(Progress::Done(flow)),
                    Err(error) => Err(CaptureFailure {
                        error,
                        notified: true,
                    }),
                };
            }
            if recording.is_recording() {
                // 录制时屏幕上没有遮罩，用户不一定记得还在录屏：通知并告诉他怎么停止。
                let stop_hotkey = self
                    .config
                    .bindings
                    .iter()
                    .find(|binding| binding.capture == *recording.request())
                    .map(|binding| binding.hotkey);
                return Err(CaptureFailure::before_actions(
                    CoreError::RecordingInProgress { stop_hotkey },
                ));
            }
            // 还在框选，遮罩正显示着，热键触发时不再通知。
            return Err(CaptureFailure {
                error: CoreError::CaptureInProgress,
                notified: true,
            });
        }
        if self.region_capture.is_some() {
            // 遮罩正显示着，用户看得到这次捕获还没结束，热键触发时不再通知。
            return Err(CaptureFailure {
                error: CoreError::CaptureInProgress,
                notified: true,
            });
        }
        ensure_implemented(request).map_err(CaptureFailure::before_actions)?;
        if request.actions.contains(&PostCaptureAction::Record) {
            let recording = Recording::start(&mut self.platform, request.clone())
                .map_err(CaptureFailure::before_actions)?;
            self.recording = Some(recording);
            return Ok(Progress::Pending);
        }
        if request.mode == CaptureMode::Region {
            let capture = RegionCapture::start(&mut self.platform, request.clone())
                .map_err(CaptureFailure::before_actions)?;
            self.region_capture = Some(capture);
            return Ok(Progress::Pending);
        }
        let screenshot = capture::capture(&self.platform, request.mode)
            .map_err(CaptureFailure::before_actions)?;
        action::run(&mut self.platform, &screenshot, request)
            .map_err(|error| CaptureFailure::in_actions(error, request))?;
        Ok(Progress::Done(Flow::Continue))
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
            Command::Capture(request) => self.capture(&request).map_err(|failure| failure.error),
            Command::Activate => Ok(Progress::Done(Flow::Continue)),
            Command::Exit => {
                // 正在录制的内容先存好再退出；还在框选时相当于取消。
                let _ = self.stop_recording();
                self.abandon_region_capture();
                self.unregister_hotkeys();
                Ok(Progress::Done(Flow::Exit))
            }
        }
    }

    /// 把遮罩上的输入交给进行中的区域捕获。捕获结束时回复等待中的请求；
    /// 热键触发的捕获失败时通知用户。
    fn on_overlay_input(&mut self, input: &OverlayInput) -> Result<Flow, CoreError> {
        if self.recording.is_some() {
            return self.advance_recording(|recording, platform| {
                recording.on_overlay_input(platform, input)
            });
        }
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
        match (requester, ending) {
            (Some(requester), ending) => {
                let reply = match &ending {
                    Ok(Ending::Captured) => Reply::Done,
                    Ok(Ending::Cancelled) => Reply::Cancelled,
                    Err(failure) => reply_for_error(&failure.error),
                };
                requester.reply(reply.encode());
                ending.map(|_| Flow::Continue).map_err(|f| f.error)
            }
            (None, Ok(_)) => Ok(Flow::Continue),
            (None, Err(failure)) => {
                self.notify_capture_failure(&failure);
                Err(failure.error)
            }
        }
    }

    /// 用 `step` 推进进行中的录屏（遮罩上的输入、节拍、停止）；录屏因此结束时收尾。
    fn advance_recording(
        &mut self,
        step: impl FnOnce(
            &mut Recording,
            &mut Platform,
        ) -> Option<Result<recording::Ending, CaptureFailure>>,
    ) -> Result<Flow, CoreError> {
        let Some(recording) = self.recording.as_mut() else {
            return Ok(Flow::Continue);
        };
        let Some(ending) = step(recording, &mut self.platform) else {
            return Ok(Flow::Continue);
        };
        let requester = self
            .recording
            .take()
            .and_then(|recording| recording.requester);
        match (requester, ending) {
            (Some(requester), ending) => {
                let reply = match &ending {
                    Ok(recording::Ending::Saved) => Reply::Done,
                    Ok(recording::Ending::Cancelled) => Reply::Cancelled,
                    Err(failure) => reply_for_error(&failure.error),
                };
                requester.reply(reply.encode());
                ending.map(|_| Flow::Continue).map_err(|f| f.error)
            }
            (None, Ok(_)) => Ok(Flow::Continue),
            (None, Err(failure)) => {
                self.notify_capture_failure(&failure);
                Err(failure.error)
            }
        }
    }

    /// 停止进行中的录屏：正在录制时存储已录下的内容，还在框选时取消。
    fn stop_recording(&mut self) -> Result<Flow, CoreError> {
        self.advance_recording(|recording, platform| Some(recording.stop(platform)))
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

    /// 启动时读取配置文件；文件不存在时写入默认配置。出错时通知用户并使用默认配置。
    fn load_config_at_startup(&mut self) -> Config {
        match self.platform.config_file.read() {
            Ok(None) => {
                if let Err(e) = self.platform.config_file.write(DEFAULT_CONFIG) {
                    self.notify_with_config_link(
                        "无法写入配置文件",
                        format!("本次使用默认配置。{e}"),
                    );
                }
                Config::default()
            }
            Ok(Some(text)) => match Config::parse(&text) {
                Ok(config) => config,
                Err(e) => {
                    self.notify_with_config_link(
                        "配置文件有错误",
                        format!("本次暂时使用默认配置。{e}"),
                    );
                    Config::default()
                }
            },
            Err(e) => {
                self.notify_with_config_link(
                    "无法读取配置文件",
                    format!("本次暂时使用默认配置。{e}"),
                );
                Config::default()
            }
        }
    }

    /// 配置文件被外部修改后重新加载，只重做变化了的部分：热键绑定变了就重新注册热键，
    /// 开机自启相关的设置变了就重新登记；刚开启“以管理员身份运行”时以管理员身份重新启动。
    ///
    /// 内容等价（例如只改了注释，或编辑器原样保存）时什么都不做，不会重复通知。
    fn reload_config(&mut self) -> Flow {
        let text = match self.platform.config_file.read() {
            Ok(Some(text)) => text,
            Ok(None) => {
                self.notify_with_config_link(
                    "配置文件已被删除",
                    "继续使用当前的配置；重启 BetterPrtSc 后会重新生成默认配置。".to_owned(),
                );
                return Flow::Continue;
            }
            Err(e) => {
                self.notify_with_config_link(
                    "无法读取配置文件",
                    format!("继续使用上一次有效的配置。{e}"),
                );
                return Flow::Continue;
            }
        };
        let config = match Config::parse(&text) {
            Ok(config) => config,
            Err(e) => {
                self.notify_with_config_link(
                    "配置文件有错误",
                    format!("继续使用上一次有效的配置。{e}"),
                );
                return Flow::Continue;
            }
        };
        let old = std::mem::replace(&mut self.config, config);
        // 只在刚开启时请求一次管理员权限：用户拒绝后，之后无关的配置修改不再反复弹 UAC 提示。
        if !old.run_as_admin
            && self.config.run_as_admin
            && !self.platform.elevation.is_elevated()
            && self.restart_as_administrator()
        {
            self.unregister_hotkeys();
            return Flow::Exit;
        }
        if old.bindings != self.config.bindings {
            let _ = self.register_hotkeys();
        }
        if (old.autostart, old.run_as_admin) != (self.config.autostart, self.config.run_as_admin) {
            self.apply_autostart();
        }
        Flow::Continue
    }

    /// 以管理员身份启动接替本实例的新实例，返回新实例是否已经启动。
    /// 用户拒绝或启动失败时发通知，本实例继续以普通权限运行。
    fn restart_as_administrator(&mut self) -> bool {
        match self.platform.elevation.restart_as_administrator() {
            Ok(()) => {
                self.restarting_as_administrator = true;
                true
            }
            Err(e) => {
                self.notify_not_elevated(&format!("{e}。"));
                false
            }
        }
    }

    fn notify_not_elevated(&self, reason: &str) {
        self.platform.notifier.notify(Notification {
            title: "未能以管理员身份运行".to_owned(),
            body: format!("{reason}本次以普通权限运行：管理员权限的窗口在前台时，热键不会生效。"),
            on_click: None,
        });
    }

    /// 按配置登记或移除开机自启。只动指向本程序的登记，别处另一份 BetterPrtSc 的登记
    /// 只在本程序要开启同一种方式时才被覆盖。
    ///
    /// 以管理员身份运行时用 [`AutostartMethod::Elevated`]（登录时不弹 UAC 提示），否则用
    /// [`AutostartMethod::Normal`]。前者需要管理员权限才能登记：本进程还没有管理员权限时
    /// （例如用户拒绝了 UAC 提示）保持现状——原有的普通方式登记仍能在登录时启动程序，
    /// 等以管理员身份运行后再改过来。
    fn apply_autostart(&mut self) {
        let wanted = self
            .config
            .autostart
            .then_some(if self.config.run_as_admin {
                AutostartMethod::Elevated
            } else {
                AutostartMethod::Normal
            });
        if wanted == Some(AutostartMethod::Elevated) && !self.platform.elevation.is_elevated() {
            return;
        }
        let entries = match self.platform.autostart.entries() {
            Ok(entries) => entries,
            Err(e) => return self.notify_plain("无法读取开机自启状态", &e),
        };
        if let Some(method) = wanted
            && !entries.iter().any(|e| e.method == method && e.this_program)
        {
            match self.platform.autostart.enable(method) {
                Ok(()) if self.options.portable => self.platform.notifier.notify(Notification {
                    title: "已开启开机自启".to_owned(),
                    body: "便携模式下，开机自启指向当前位置的程序：移动程序目录后需要重新开启\
                           （从新位置启动一次 BetterPrtSc 即可）。"
                        .to_owned(),
                    on_click: None,
                }),
                Ok(()) => {}
                // 新的登记没成功时保留原有的登记，至少还能开机自启。
                Err(e) => return self.notify_plain("无法开启开机自启", &e),
            }
        }
        for entry in entries {
            if entry.this_program
                && Some(entry.method) != wanted
                && let Err(e) = self.platform.autostart.disable(entry.method)
            {
                self.notify_plain("无法关闭开机自启", &e);
            }
        }
    }

    /// 发出一条点击后什么也不做、正文就是错误说明的通知。
    fn notify_plain(&self, title: &str, error: &PlatformError) {
        self.platform.notifier.notify(Notification {
            title: title.to_owned(),
            body: error.to_string(),
            on_click: None,
        });
    }

    /// 发出一条点击后打开配置文件的通知。
    fn notify_with_config_link(&self, title: &str, body: String) {
        let path = self.platform.config_file.path().to_path_buf();
        self.platform.notifier.notify(Notification {
            title: title.to_owned(),
            body,
            on_click: Some(NotificationClick::OpenFile(path)),
        });
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
        CoreError::UnsupportedCaptureMode(_)
        | CoreError::UnsupportedAction(_)
        | CoreError::UnsupportedRecordingFormat(_) => Reply::Unsupported(error.to_string()),
        _ => Reply::Failed(error.to_string()),
    }
}

/// 尚未实现的捕获模式或截后动作在捕获开始之前就拒绝整个请求，不产生任何产物。
fn ensure_implemented(request: &CaptureRequest) -> Result<(), CoreError> {
    if !capture::is_implemented(request.mode) {
        return Err(CoreError::UnsupportedCaptureMode(request.mode));
    }
    match request
        .actions
        .iter()
        .find(|a| !action::is_implemented(**a))
    {
        Some(action) => Err(CoreError::UnsupportedAction(*action)),
        None => Ok(()),
    }
}

/// 不带修饰键的 PrtSc：系统截图工具占用的只有它。
const PRINT_SCREEN: Hotkey = Hotkey::new(Modifiers::NONE, Key::PrintScreen);

/// 第 `index` 条绑定使用的热键标识。
fn hotkey_id(index: usize) -> HotkeyId {
    HotkeyId(index as u32 + 1)
}
