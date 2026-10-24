use bps_platform::{
    HotkeyId, IncomingRequest, LatestRelease, Notification, NotificationClick, Platform,
    PlatformError, PlatformEvent, RegisterHotkeyError, StandaloneWindow, TrayMenuItem,
    TrayMenuItemId,
};

use crate::{
    BindingFailure, CaptureRequest, Command, Config, CoreError, DEFAULT_CONFIG, Flow,
    HotkeyBinding, HotkeyRegistrationFailure, Locale, PostCaptureAction, RegistrationError, Reply,
    Request, ResidentStatus, Texts, UpdateStatus, VERSION, action, capture, update,
};

/// 托盘菜单“设置”。
const MENU_SETTINGS: TrayMenuItemId = TrayMenuItemId(1);
/// 托盘菜单“退出”。
const MENU_EXIT: TrayMenuItemId = TrayMenuItemId(2);

/// 常驻实例的核心：持有平台能力和配置，把触发事件转换成命令并执行。
///
/// 宿主（`app` 里的平台事件循环）用 [`start`](Self::start) 创建它，把
/// [`PlatformEvent`] 交给 [`handle`](Self::handle)，在返回 [`Flow::Exit`] 时退出循环。
pub struct ResidentInstance {
    platform: Platform,
    config: Config,
    /// 界面语言（由配置和系统语言决定），托盘菜单和通知都用它。
    locale: Locale,
    /// 已成功注册的热键。
    registered: Vec<HotkeyId>,
    /// 最近一次注册热键时失败的绑定。
    failures: Vec<HotkeyRegistrationFailure>,
    /// 检查更新的状态。
    update: UpdateStatus,
    /// 本次运行中已经提示过的新版本，同一个版本只提示一次。
    announced_version: Option<String>,
}

impl ResidentInstance {
    /// 用给定的热键绑定创建常驻实例，不读写配置文件、不注册热键、不设置托盘菜单。
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        let config = Config::with_bindings(bindings);
        let locale = config
            .language
            .resolve(&platform.system_language.preferred_ui_languages());
        Self {
            platform,
            config,
            locale,
            registered: Vec::new(),
            failures: Vec::new(),
            update: UpdateStatus::Disabled,
            announced_version: None,
        }
    }

    /// 启动常驻实例：读取配置文件（不存在时写入默认配置），按界面语言设置托盘菜单，
    /// 注册热键，并开始监视配置文件；开启了检查更新时查询一次最新版本。
    ///
    /// 不会失败：配置文件有错误、热键注册失败等问题都通过通知告诉用户，
    /// 有问题的部分用默认值代替或跳过。
    pub fn start(platform: Platform) -> Self {
        let mut instance = Self::new(platform, Vec::new());
        // 配置出错时的通知用系统语言（`new` 已按系统语言选好）。
        instance.config = instance.load_config_at_startup();
        instance.apply_language();
        let _ = instance.register_hotkeys();
        if let Err(e) = instance.platform.config_file.watch() {
            let texts = instance.texts();
            instance.notify_with_config_link(
                texts.get("notification-config-unwatchable-title"),
                texts.format("notification-config-unwatchable-body", &[("error", &e)]),
            );
        }
        if instance.config.check_for_updates {
            instance.check_for_updates();
        }
        instance
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
    /// 失败的绑定不影响其他绑定；有失败时发出一条通知，并可用
    /// [`registration_failures`](Self::registration_failures) 查询。
    pub fn register_hotkeys(&mut self) -> &[HotkeyRegistrationFailure] {
        self.unregister_hotkeys();
        self.failures.clear();
        let bindings = &self.config.bindings;
        for (index, binding) in bindings.iter().enumerate() {
            let error = match bindings[..index]
                .iter()
                .position(|earlier| earlier.hotkey == binding.hotkey)
            {
                Some(first) => RegistrationError::DuplicateOf(first),
                None => match self
                    .platform
                    .hotkeys
                    .register(hotkey_id(index), binding.hotkey)
                {
                    Ok(()) => {
                        self.registered.push(hotkey_id(index));
                        continue;
                    }
                    Err(RegisterHotkeyError::TakenByOtherProgram) => {
                        RegistrationError::TakenByOtherProgram
                    }
                    Err(RegisterHotkeyError::Failed(e)) => RegistrationError::Platform(e),
                },
            };
            self.failures.push(HotkeyRegistrationFailure {
                index,
                binding: binding.clone(),
                error,
            });
        }
        if !self.failures.is_empty() {
            let texts = self.texts();
            let body = self
                .failures
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
        &self.failures
    }

    /// 把平台事件转换成命令；与任何绑定都无关的事件、无法识别的请求返回 `None`。
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
            PlatformEvent::ConfigFileChanged | PlatformEvent::LatestReleaseFetched(_) => None,
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).and_then(|r| self.command_for_request(&r))
            }
        }
    }

    /// 处理一个平台事件：配置文件变更时重新加载配置；请求事件执行后把结果回复给发起方；
    /// 其余事件转换成命令后执行，失败时发通知。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match event {
            PlatformEvent::ConfigFileChanged => {
                self.reload_config();
                return Ok(Flow::Continue);
            }
            PlatformEvent::RequestReceived(request) => return self.serve(request),
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
            Ok(()) => Ok(Flow::Continue),
            Err(CaptureFailure { error, notified }) => {
                if !notified {
                    let texts = self.texts();
                    let title = match error {
                        CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_) => {
                            "notification-unsupported-title"
                        }
                        _ => "notification-capture-failed-title",
                    };
                    self.platform.notifier.notify(Notification {
                        title: texts.get(title),
                        body: error.localized(texts),
                        on_click: None,
                    });
                }
                Err(error)
            }
        }
    }

    /// 执行转交来的请求，并把结果回复给发起方。状态查询只读取、不执行命令。
    fn serve(&mut self, incoming: IncomingRequest) -> Result<Flow, CoreError> {
        let request = Request::decode(incoming.message());
        let result = match request.as_ref().map(|r| (r, self.command_for_request(r))) {
            Some((_, Some(command))) => self.execute(command),
            Some((Request::Status, None)) => {
                incoming.reply(Reply::Status(self.status()).encode());
                return Ok(Flow::Continue);
            }
            Some((_, None)) | None => Err(CoreError::InvalidRequest(incoming.message().to_owned())),
        };
        let texts = self.texts();
        let reply = match &result {
            Ok(_) => Reply::Done,
            Err(e @ (CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_))) => {
                Reply::Unsupported(e.localized(texts))
            }
            Err(e) => Reply::Failed(e.localized(texts)),
        };
        incoming.reply(reply.encode());
        result
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
    fn capture(&mut self, request: &CaptureRequest) -> Result<(), CaptureFailure> {
        let not_notified = |error| CaptureFailure {
            error,
            notified: false,
        };
        ensure_implemented(request).map_err(not_notified)?;
        let screenshot = capture::capture(&self.platform, request.mode).map_err(not_notified)?;
        let texts = self.texts();
        action::run(&mut self.platform, &screenshot, request, texts).map_err(|error| {
            CaptureFailure {
                error,
                // 做了自动存储时，截后动作已经就结果发过通知（“截图已保存”或“自动存储失败”）。
                notified: request.actions.contains(&PostCaptureAction::AutoSave),
            }
        })
    }

    /// 执行一条命令。
    pub fn execute(&mut self, command: Command) -> Result<Flow, CoreError> {
        match command {
            Command::Capture(request) => {
                self.capture(&request).map_err(|failure| failure.error)?;
                Ok(Flow::Continue)
            }
            Command::Activate | Command::OpenSettings => {
                self.open_settings()?;
                Ok(Flow::Continue)
            }
            Command::Exit => {
                self.unregister_hotkeys();
                Ok(Flow::Exit)
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

    /// 配置文件被外部（或设置界面）修改后重新加载。
    ///
    /// 内容等价（例如只改了注释，或编辑器原样保存）时什么都不做，不会重复通知；
    /// 界面语言变了就更新托盘菜单；热键绑定变了才重新注册热键。
    fn reload_config(&mut self) {
        let texts = self.texts();
        let fallback = |error: &dyn std::fmt::Display| {
            texts.format("notification-config-reload-fallback", &[("error", error)])
        };
        let text = match self.platform.config_file.read() {
            Ok(Some(text)) => text,
            Ok(None) => {
                return self.notify_with_config_link(
                    texts.get("notification-config-deleted-title"),
                    texts.get("notification-config-deleted-body"),
                );
            }
            Err(e) => {
                return self.notify_with_config_link(
                    texts.get("notification-config-unreadable-title"),
                    fallback(&e),
                );
            }
        };
        let config = match Config::parse(&text) {
            Ok(config) => config,
            Err(e) => {
                return self.notify_with_config_link(
                    texts.get("notification-config-invalid-title"),
                    fallback(&e.localized(texts)),
                );
            }
        };
        if config == self.config {
            return;
        }
        let old = std::mem::replace(&mut self.config, config);
        self.apply_language();
        if old.bindings != self.config.bindings {
            let _ = self.register_hotkeys();
        }
        match (old.check_for_updates, self.config.check_for_updates) {
            (false, true) => self.check_for_updates(),
            (true, false) => self.update = UpdateStatus::Disabled,
            _ => {}
        }
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

    /// 当前界面语言的文本。
    fn texts(&self) -> &'static Texts {
        Texts::of(self.locale)
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

/// 第 `index` 条绑定使用的热键标识。
fn hotkey_id(index: usize) -> HotkeyId {
    HotkeyId(index as u32 + 1)
}

/// 一次捕获失败的原因，以及截后动作是否已经就这次失败通知过用户。
struct CaptureFailure {
    error: CoreError,
    notified: bool,
}
