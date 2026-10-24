use bps_platform::{
    AutostartMethod, Hotkey, HotkeyId, IncomingRequest, Key, LatestRelease, Modifiers,
    Notification, NotificationClick, OverlayInput, Platform, PlatformError, PlatformEvent,
    RegisterHotkeyError, StandaloneWindow, TrayMenuItem, TrayMenuItemId,
};

use crate::capture::CaptureFailure;
use crate::region::{Ending, RegionCapture};
use crate::{
    BindingFailure, CaptureMode, CaptureRequest, Command, Config, CoreError, DEFAULT_CONFIG, Flow,
    HotkeyBinding, HotkeyRegistrationFailure, Locale, PostCaptureAction, RegistrationError, Reply,
    Request, ResidentStatus, Texts, UpdateStatus, VERSION, action, capture, update,
};

/// 托盘菜单“设置”。
const MENU_SETTINGS: TrayMenuItemId = TrayMenuItemId(1);
/// 托盘菜单“退出”。
const MENU_EXIT: TrayMenuItemId = TrayMenuItemId(2);

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
pub struct ResidentInstance {
    platform: Platform,
    config: Config,
    options: StartOptions,
    /// 界面语言（由配置和系统语言决定），托盘菜单和通知都用它。
    locale: Locale,
    /// 已成功注册的热键。
    registered: Vec<HotkeyId>,
    /// 最近一次注册热键时失败的绑定。
    failures: Vec<HotkeyRegistrationFailure>,
    /// 已以管理员身份启动了接替本实例的新实例。
    restarting_as_administrator: bool,
    /// 检查更新的状态。
    update: UpdateStatus,
    /// 本次运行中已经提示过的新版本，同一个版本只提示一次。
    announced_version: Option<String>,
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
    /// 用给定的热键绑定创建常驻实例，不读写配置文件、不注册热键、不动开机自启、
    /// 不设置托盘菜单。
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        let config = Config::with_bindings(bindings);
        let locale = config
            .language
            .resolve(&platform.system_language.preferred_ui_languages());
        Self {
            platform,
            config,
            options: StartOptions::default(),
            locale,
            registered: Vec::new(),
            failures: Vec::new(),
            restarting_as_administrator: false,
            update: UpdateStatus::Disabled,
            announced_version: None,
            region_capture: None,
        }
    }

    /// 以默认的运行环境（非便携模式）启动常驻实例，见 [`start_with`](Self::start_with)。
    pub fn start(platform: Platform) -> Self {
        Self::start_with(platform, StartOptions::default())
    }

    /// 启动常驻实例：读取配置文件（不存在时写入默认配置），按界面语言设置托盘菜单；
    /// 配置要求以管理员身份运行而本进程没有管理员权限时，以管理员身份重新启动（见
    /// [`restarting_as_administrator`](Self::restarting_as_administrator)）；否则注册热键，
    /// 按配置登记或移除开机自启，开始监视配置文件和 PrtSc 键的系统设置，
    /// 开启了检查更新时查询一次最新版本。
    ///
    /// 不会失败：配置文件有错误、热键注册失败等问题都通过通知告诉用户，
    /// 有问题的部分用默认值代替或跳过。
    pub fn start_with(platform: Platform, options: StartOptions) -> Self {
        let mut instance = Self::new(platform, Vec::new());
        instance.options = options;
        // 配置出错时的通知用系统语言（`new` 已按系统语言选好）。
        instance.config = instance.load_config_at_startup();
        instance.apply_language();
        if instance.config.run_as_admin && !instance.platform.elevation.is_elevated() {
            if options.restarted_as_administrator {
                let reason = instance.texts().get("not-elevated-after-restart");
                instance.notify_not_elevated(&reason);
            } else if instance.restart_as_administrator() {
                return instance;
            }
        }
        let _ = instance.register_hotkeys();
        instance.apply_autostart();
        if let Err(e) = instance.platform.config_file.watch() {
            let texts = instance.texts();
            instance.notify_with_config_link(
                texts.get("notification-config-unwatchable-title"),
                texts.format("notification-config-unwatchable-body", &[("error", &e)]),
            );
        }
        // 监视失败只是少了“关闭系统设置后立即接管 PrtSc”，重启后照样生效，不打扰用户。
        let _ = instance.platform.system_screenshot_tool.watch();
        if instance.config.check_for_updates {
            instance.check_for_updates();
        }
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

    /// 设置界面经 [`Request::Status`] 查询到的状态。
    pub fn status(&self) -> ResidentStatus {
        ResidentStatus {
            registration_failures: self
                .failures
                .iter()
                .map(|f| BindingFailure {
                    index: f.index,
                    hotkey: f.binding.hotkey,
                    error: f.error.clone(),
                })
                .collect(),
            update: self.update.clone(),
        }
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
            Err(RegisterHotkeyError::TakenByOtherProgram) => {
                Err(RegistrationError::TakenByOtherProgram)
            }
            Err(RegisterHotkeyError::Failed(e)) => Err(RegistrationError::Platform(e)),
        }
    }

    /// 为注册失败的绑定发通知：系统截图工具占用 PrtSc 的发引导通知，其余的汇总成一条。
    fn notify_registration_failures(&self, failures: &[HotkeyRegistrationFailure]) {
        let texts = self.texts();
        let (taken_by_system, others): (Vec<_>, Vec<_>) = failures
            .iter()
            .partition(|f| f.error == RegistrationError::TakenBySystemScreenshotTool);
        if !others.is_empty() {
            let body = others
                .iter()
                .map(|f| {
                    texts.format(
                        "notification-hotkey-failed-line",
                        &[
                            ("hotkey", &f.binding.hotkey),
                            ("reason", &f.error.localized(texts)),
                        ],
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.notify_with_config_link(texts.get("notification-hotkey-failed-title"), body);
        }
        if !taken_by_system.is_empty() {
            self.platform.notifier.notify(Notification {
                title: texts.get("notification-prtsc-taken-title"),
                body: texts.get("notification-prtsc-taken-body"),
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
            let texts = self.texts();
            self.platform.notifier.notify(Notification {
                title: texts.get("notification-prtsc-available-title"),
                body: texts.get("notification-prtsc-available-body"),
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
            PlatformEvent::TrayMenuItemSelected(MENU_SETTINGS) => Some(Command::OpenSettings),
            PlatformEvent::TrayMenuItemSelected(MENU_EXIT) => Some(Command::Exit),
            PlatformEvent::TrayMenuItemSelected(_) => None,
            PlatformEvent::ConfigFileChanged
            | PlatformEvent::PrintScreenKeySettingChanged
            | PlatformEvent::LatestReleaseFetched(_)
            | PlatformEvent::OverlayInput(_) => None,
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).and_then(|r| self.command_for_request(&r))
            }
        }
    }

    /// 处理一个平台事件：配置文件变更时重新加载配置；PrtSc 键的系统设置变化时重试被系统
    /// 截图工具挡住的热键绑定；遮罩上的输入交给进行中的区域捕获；请求事件执行后把结果回复给
    /// 发起方（需要用户操作的捕获等捕获结束再回复）；检查更新有结果时按需提示；其余事件转换成
    /// 命令后执行，失败时发通知。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match event {
            PlatformEvent::ConfigFileChanged => return Ok(self.reload_config()),
            PlatformEvent::PrintScreenKeySettingChanged => {
                self.retry_print_screen_bindings();
                return Ok(Flow::Continue);
            }
            PlatformEvent::RequestReceived(request) => return self.serve(request),
            PlatformEvent::OverlayInput(input) => return self.on_overlay_input(&input),
            PlatformEvent::LatestReleaseFetched(result) => {
                self.latest_release_fetched(result);
                return Ok(Flow::Continue);
            }
            PlatformEvent::HotkeyPressed(_)
            | PlatformEvent::ExitRequested
            | PlatformEvent::TrayMenuItemSelected(_) => {}
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
        let texts = self.texts();
        let title = match failure.error {
            CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_) => {
                "notification-unsupported-title"
            }
            _ => "notification-capture-failed-title",
        };
        self.platform.notifier.notify(Notification {
            title: texts.get(title),
            body: failure.error.localized(texts),
            on_click: None,
        });
    }

    /// 执行转交来的请求，并把结果回复给发起方；需要用户操作的捕获等捕获结束再回复。
    /// 状态查询只读取、不执行命令。
    fn serve(&mut self, incoming: IncomingRequest) -> Result<Flow, CoreError> {
        let request = Request::decode(incoming.message());
        let result = match request.as_ref().map(|r| (r, self.command_for_request(r))) {
            Some((_, Some(command))) => self.run(command),
            Some((Request::Status, None)) => {
                incoming.reply(Reply::Status(self.status()).encode());
                return Ok(Flow::Continue);
            }
            Some((_, None)) | None => Err(CoreError::InvalidRequest(incoming.message().to_owned())),
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
                incoming.reply(reply_for_error(&e, self.texts()).encode());
                Err(e)
            }
        }
    }

    /// 请求对应的命令；只读的状态查询没有命令。`capture <捕获模式>` 与按下该捕获模式的热键
    /// 完全一致：采用第一条该捕获模式的热键绑定；没有这样的绑定时只复制到剪贴板。
    fn command_for_request(&self, request: &Request) -> Option<Command> {
        Some(match request {
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
            Request::Status => return None,
        })
    }

    /// 执行一次捕获：先确认捕获模式和截后动作都已实现，再捕获、执行截后动作。
    ///
    /// 区域捕获只拍下冻结帧、显示选区遮罩（[`Progress::Pending`]），之后由遮罩上的输入推进。
    fn capture(&mut self, request: &CaptureRequest) -> Result<Progress, CaptureFailure> {
        if self.region_capture.is_some() {
            // 遮罩正显示着，用户看得到这次捕获还没结束，热键触发时不再通知。
            return Err(CaptureFailure {
                error: CoreError::CaptureInProgress,
                notified: true,
            });
        }
        ensure_implemented(request).map_err(CaptureFailure::before_actions)?;
        let texts = self.texts();
        if request.mode == CaptureMode::Region {
            let capture = RegionCapture::start(&mut self.platform, request.clone(), texts)
                .map_err(CaptureFailure::before_actions)?;
            self.region_capture = Some(capture);
            return Ok(Progress::Pending);
        }
        let screenshot = capture::capture(&self.platform, request.mode)
            .map_err(CaptureFailure::before_actions)?;
        action::run(&mut self.platform, screenshot, request, texts)?;
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
            Command::Activate | Command::OpenSettings => {
                self.open_settings()?;
                Ok(Progress::Done(Flow::Continue))
            }
            Command::Exit => {
                self.abandon_region_capture();
                self.unregister_hotkeys();
                Ok(Progress::Done(Flow::Exit))
            }
        }
    }

    /// 把遮罩上的输入交给进行中的区域捕获。捕获结束时回复等待中的请求；
    /// 热键触发的捕获失败时通知用户。
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
        match (requester, ending) {
            (Some(requester), ending) => {
                let reply = match &ending {
                    Ok(Ending::Captured) => Reply::Done,
                    Ok(Ending::Cancelled) => Reply::Cancelled,
                    Err(failure) => reply_for_error(&failure.error, self.texts()),
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

    /// 放弃进行中的区域捕获：关闭遮罩，等待中的请求收到“已取消”。
    fn abandon_region_capture(&mut self) {
        if let Some(capture) = self.region_capture.take() {
            self.platform.overlay.close();
            if let Some(requester) = capture.requester {
                requester.reply(Reply::Cancelled.encode());
            }
        }
    }

    /// 打开设置界面；打不开时通知用户。
    fn open_settings(&mut self) -> Result<(), CoreError> {
        if let Err(e) = self.platform.windows.open(StandaloneWindow::Settings) {
            let texts = self.texts();
            self.platform.notifier.notify(Notification {
                title: texts.get("notification-settings-unavailable-title"),
                body: e.to_string(),
                on_click: None,
            });
            return Err(e.into());
        }
        Ok(())
    }

    /// 启动时读取配置文件；文件不存在时写入默认配置。出错时通知用户并使用默认配置。
    fn load_config_at_startup(&mut self) -> Config {
        let texts = self.texts();
        let fallback = |error: &dyn std::fmt::Display| {
            texts.format("notification-config-startup-fallback", &[("error", error)])
        };
        match self.platform.config_file.read() {
            Ok(None) => {
                if let Err(e) = self.platform.config_file.write(DEFAULT_CONFIG) {
                    self.notify_with_config_link(
                        texts.get("notification-config-unwritable-title"),
                        texts.format("notification-config-unwritable-body", &[("error", &e)]),
                    );
                }
                Config::default()
            }
            Ok(Some(text)) => match Config::parse(&text) {
                Ok(config) => config,
                Err(e) => {
                    self.notify_with_config_link(
                        texts.get("notification-config-invalid-title"),
                        fallback(&e.localized(texts)),
                    );
                    Config::default()
                }
            },
            Err(e) => {
                self.notify_with_config_link(
                    texts.get("notification-config-unreadable-title"),
                    fallback(&e),
                );
                Config::default()
            }
        }
    }

    /// 配置文件被外部（或设置界面）修改后重新加载，只重做变化了的部分：界面语言变了就更新
    /// 托盘菜单，热键绑定变了就重新注册热键，开机自启相关的设置变了就重新登记，检查更新
    /// 刚开启时查询一次；刚开启“以管理员身份运行”时以管理员身份重新启动。
    ///
    /// 内容等价（例如只改了注释，或编辑器原样保存）时什么都不做，不会重复通知。
    fn reload_config(&mut self) -> Flow {
        let texts = self.texts();
        let fallback = |error: &dyn std::fmt::Display| {
            texts.format("notification-config-reload-fallback", &[("error", error)])
        };
        let text = match self.platform.config_file.read() {
            Ok(Some(text)) => text,
            Ok(None) => {
                self.notify_with_config_link(
                    texts.get("notification-config-deleted-title"),
                    texts.get("notification-config-deleted-body"),
                );
                return Flow::Continue;
            }
            Err(e) => {
                self.notify_with_config_link(
                    texts.get("notification-config-unreadable-title"),
                    fallback(&e),
                );
                return Flow::Continue;
            }
        };
        let config = match Config::parse(&text) {
            Ok(config) => config,
            Err(e) => {
                self.notify_with_config_link(
                    texts.get("notification-config-invalid-title"),
                    fallback(&e.localized(texts)),
                );
                return Flow::Continue;
            }
        };
        if config == self.config {
            return Flow::Continue;
        }
        let old = std::mem::replace(&mut self.config, config);
        if old.language != self.config.language {
            self.apply_language();
        }
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
        match (old.check_for_updates, self.config.check_for_updates) {
            (false, true) => self.check_for_updates(),
            (true, false) => self.update = UpdateStatus::Disabled,
            _ => {}
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
                self.notify_not_elevated(&e.to_string());
                false
            }
        }
    }

    fn notify_not_elevated(&self, reason: &str) {
        let texts = self.texts();
        self.platform.notifier.notify(Notification {
            title: texts.get("notification-not-elevated-title"),
            body: texts.format("notification-not-elevated-body", &[("reason", &reason)]),
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
            Err(e) => return self.notify_plain("notification-autostart-unreadable-title", &e),
        };
        if let Some(method) = wanted
            && !entries.iter().any(|e| e.method == method && e.this_program)
        {
            match self.platform.autostart.enable(method) {
                Ok(()) if self.options.portable => {
                    let texts = self.texts();
                    self.platform.notifier.notify(Notification {
                        title: texts.get("notification-autostart-enabled-portable-title"),
                        body: texts.get("notification-autostart-enabled-portable-body"),
                        on_click: None,
                    })
                }
                Ok(()) => {}
                // 新的登记没成功时保留原有的登记，至少还能开机自启。
                Err(e) => {
                    return self.notify_plain("notification-autostart-enable-failed-title", &e);
                }
            }
        }
        for entry in entries {
            if entry.this_program
                && Some(entry.method) != wanted
                && let Err(e) = self.platform.autostart.disable(entry.method)
            {
                self.notify_plain("notification-autostart-disable-failed-title", &e);
            }
        }
    }

    /// 按配置的界面语言更新托盘菜单，之后的通知也用这种语言。
    fn apply_language(&mut self) {
        self.locale = self
            .config
            .language
            .resolve(&self.platform.system_language.preferred_ui_languages());
        let texts = self.texts();
        self.platform.tray_menu.set_items(vec![
            TrayMenuItem {
                id: MENU_SETTINGS,
                label: texts.get("tray-settings"),
            },
            TrayMenuItem {
                id: MENU_EXIT,
                label: texts.get("tray-exit"),
            },
        ]);
    }

    /// 查询 GitHub Releases 上的最新正式版本；结果稍后以事件递交。
    /// 只在用户开启了检查更新时调用（ADR 0004）。
    fn check_for_updates(&mut self) {
        self.update = UpdateStatus::Checking;
        self.platform
            .updates
            .request_latest_release(update::RELEASES_REPOSITORY);
    }

    /// 检查更新有了结果：有新版本时提示一次（点击打开发布页面），不下载。
    /// 用户在结果返回前关闭了检查更新时，丢弃结果。
    fn latest_release_fetched(&mut self, result: Result<Option<LatestRelease>, PlatformError>) {
        if !self.config.check_for_updates {
            return;
        }
        let texts = self.texts();
        let release = match result {
            Ok(Some(release)) => release,
            Ok(None) => return self.update = UpdateStatus::UpToDate,
            Err(e) => return self.update = UpdateStatus::Failed(e.to_string()),
        };
        match update::is_newer(&release.tag, VERSION) {
            None => {
                self.update = UpdateStatus::Failed(
                    texts.format("update-unrecognized-version", &[("version", &release.tag)]),
                );
            }
            Some(false) => self.update = UpdateStatus::UpToDate,
            Some(true) => {
                if self.announced_version.as_deref() != Some(release.tag.as_str()) {
                    self.platform.notifier.notify(Notification {
                        title: texts.format(
                            "notification-update-available-title",
                            &[("version", &release.tag)],
                        ),
                        body: texts.format(
                            "notification-update-available-body",
                            &[("current", &VERSION)],
                        ),
                        on_click: Some(NotificationClick::OpenUrl(release.page_url.clone())),
                    });
                    self.announced_version = Some(release.tag.clone());
                }
                self.update = UpdateStatus::Available {
                    version: release.tag,
                    page_url: release.page_url,
                };
            }
        }
    }

    /// 当前界面语言的文本。
    fn texts(&self) -> &'static Texts {
        Texts::of(self.locale)
    }

    /// 发出一条点击后什么也不做、正文就是错误说明的通知；`title` 是文本资源里的消息。
    fn notify_plain(&self, title: &str, error: &PlatformError) {
        self.platform.notifier.notify(Notification {
            title: self.texts().get(title),
            body: error.to_string(),
            on_click: None,
        });
    }

    /// 发出一条点击后打开配置文件的通知。
    fn notify_with_config_link(&self, title: String, body: String) {
        let path = self.platform.config_file.path().to_path_buf();
        self.platform.notifier.notify(Notification {
            title,
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

/// 命令执行失败时给请求发起方的回复，说明文字用 `texts` 的语言。
fn reply_for_error(error: &CoreError, texts: &Texts) -> Reply {
    match error {
        CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_) => {
            Reply::Unsupported(error.localized(texts))
        }
        _ => Reply::Failed(error.localized(texts)),
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
