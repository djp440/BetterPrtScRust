use bps_platform::{
    AutostartMethod, Hotkey, HotkeyId, IncomingRequest, Key, Modifiers, Notification,
    NotificationClick, Platform, PlatformError, PlatformEvent,
};

use crate::{
    CaptureRequest, Command, Config, CoreError, DEFAULT_CONFIG, Flow, HotkeyBinding,
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
            PlatformEvent::ConfigFileChanged | PlatformEvent::PrintScreenKeySettingChanged => None,
            PlatformEvent::RequestReceived(request) => {
                Request::decode(request.message()).map(|r| self.command_for_request(&r))
            }
        }
    }

    /// 处理一个平台事件：配置文件变更时重新加载配置；PrtSc 键的系统设置变化时重试被系统
    /// 截图工具挡住的热键绑定；请求事件执行后把结果回复给发起方；其余事件转换成命令后执行，
    /// 失败时发通知。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match event {
            PlatformEvent::ConfigFileChanged => return Ok(self.reload_config()),
            PlatformEvent::PrintScreenKeySettingChanged => {
                self.retry_print_screen_bindings();
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

/// 一次捕获失败的原因，以及截后动作是否已经就这次失败通知过用户。
struct CaptureFailure {
    error: CoreError,
    notified: bool,
}
