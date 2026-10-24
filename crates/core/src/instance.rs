use bps_platform::{
    HotkeyId, IncomingRequest, Notification, NotificationClick, Platform, PlatformEvent,
};

use crate::{
    CaptureRequest, Command, Config, CoreError, DEFAULT_CONFIG, Flow, HotkeyBinding,
    HotkeyRegistrationFailure, PostCaptureAction, RegistrationError, Reply, Request, action,
    capture,
};

/// 常驻实例的核心：持有平台能力和配置，把触发事件转换成命令并执行。
///
/// 宿主（`app` 里的平台事件循环）用 [`start`](Self::start) 创建它，把
/// [`PlatformEvent`] 交给 [`handle`](Self::handle)，在返回 [`Flow::Exit`] 时退出循环。
pub struct ResidentInstance {
    platform: Platform,
    config: Config,
    /// 已成功注册的热键。
    registered: Vec<HotkeyId>,
    /// 最近一次注册热键时失败的绑定。
    failures: Vec<HotkeyRegistrationFailure>,
}

impl ResidentInstance {
    /// 用给定的热键绑定创建常驻实例，不读写配置文件、不注册热键。
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            platform,
            config: Config::with_bindings(bindings),
            registered: Vec::new(),
            failures: Vec::new(),
        }
    }

    /// 启动常驻实例：读取配置文件（不存在时写入默认配置），注册热键，并开始监视配置文件。
    ///
    /// 不会失败：配置文件有错误、热键注册失败等问题都通过通知告诉用户，
    /// 有问题的部分用默认值代替或跳过。
    pub fn start(platform: Platform) -> Self {
        let mut instance = Self::new(platform, Vec::new());
        instance.config = instance.load_config_at_startup();
        let _ = instance.register_hotkeys();
        if let Err(e) = instance.platform.config_file.watch() {
            instance.notify_with_config_link(
                "无法监视配置文件",
                format!("修改配置文件后需要重启 BetterPrtSc 才能生效。{e}"),
            );
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
                    Err(e) => RegistrationError::Platform(e),
                },
            };
            self.failures.push(HotkeyRegistrationFailure {
                index,
                binding: binding.clone(),
                error,
            });
        }
        if !self.failures.is_empty() {
            let body = self
                .failures
                .iter()
                .map(|f| format!("{}：{}", f.binding.hotkey, f.error))
                .collect::<Vec<_>>()
                .join("\n");
            self.notify_with_config_link("热键注册失败", body);
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
            PlatformEvent::ConfigFileChanged => None,
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).map(|r| self.command_for_request(&r))
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
            Ok(()) => Ok(Flow::Continue),
            Err(CaptureFailure { error, notified }) => {
                if !notified {
                    let title = match error {
                        CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_) => {
                            "暂不支持"
                        }
                        _ => "捕获失败",
                    };
                    self.platform.notifier.notify(Notification {
                        title: title.to_owned(),
                        body: error.to_string(),
                        on_click: None,
                    });
                }
                Err(error)
            }
        }
    }

    /// 执行转交来的请求，并把结果回复给发起方。
    fn serve(&mut self, incoming: IncomingRequest) -> Result<Flow, CoreError> {
        let result = match Request::decode(incoming.message()) {
            Some(request) => {
                let command = self.command_for_request(&request);
                self.execute(command)
            }
            None => Err(CoreError::InvalidRequest(incoming.message().to_owned())),
        };
        let reply = match &result {
            Ok(_) => Reply::Done,
            Err(e @ (CoreError::UnsupportedCaptureMode(_) | CoreError::UnsupportedAction(_))) => {
                Reply::Unsupported(e.to_string())
            }
            Err(e) => Reply::Failed(e.to_string()),
        };
        incoming.reply(reply.encode());
        result
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
    fn capture(&mut self, request: &CaptureRequest) -> Result<(), CaptureFailure> {
        let not_notified = |error| CaptureFailure {
            error,
            notified: false,
        };
        ensure_implemented(request).map_err(not_notified)?;
        let screenshot = capture::capture(&self.platform, request.mode).map_err(not_notified)?;
        action::run(&mut self.platform, &screenshot, request).map_err(|error| CaptureFailure {
            error,
            // 做了自动存储时，截后动作已经就结果发过通知（“截图已保存”或“自动存储失败”）。
            notified: request.actions.contains(&PostCaptureAction::AutoSave),
        })
    }

    /// 执行一条命令。
    pub fn execute(&mut self, command: Command) -> Result<Flow, CoreError> {
        match command {
            Command::Capture(request) => {
                self.capture(&request).map_err(|failure| failure.error)?;
                Ok(Flow::Continue)
            }
            Command::Activate => Ok(Flow::Continue),
            Command::Exit => {
                self.unregister_hotkeys();
                Ok(Flow::Exit)
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

    /// 配置文件被外部修改后重新加载并重新注册热键。
    ///
    /// 内容等价（例如只改了注释，或编辑器原样保存）时什么都不做，不会重复通知。
    fn reload_config(&mut self) {
        let text = match self.platform.config_file.read() {
            Ok(Some(text)) => text,
            Ok(None) => {
                return self.notify_with_config_link(
                    "配置文件已被删除",
                    "继续使用当前的配置；重启 BetterPrtSc 后会重新生成默认配置。".to_owned(),
                );
            }
            Err(e) => {
                return self.notify_with_config_link(
                    "无法读取配置文件",
                    format!("继续使用上一次有效的配置。{e}"),
                );
            }
        };
        match Config::parse(&text) {
            Ok(config) if config == self.config => {}
            Ok(config) => {
                self.config = config;
                let _ = self.register_hotkeys();
            }
            Err(e) => self.notify_with_config_link(
                "配置文件有错误",
                format!("继续使用上一次有效的配置。{e}"),
            ),
        }
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
