//! 开机自启：普通方式写注册表 `HKCU\...\Run` 项；以管理员身份时改用登录触发、以最高权限
//! 运行的计划任务，登录时不弹 UAC 提示。计划任务的登记和移除都需要管理员权限。

use std::path::{Path, PathBuf};

use bps_platform::{Autostart, AutostartEntry, AutostartMethod, PlatformError};
use windows::Win32::Foundation::{E_ACCESSDENIED, ERROR_FILE_NOT_FOUND, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;
use windows::Win32::System::TaskScheduler::{
    IExecAction, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN,
    TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, Interface};

use crate::error::win_error;
use crate::registry;
use crate::token::{current_program, current_user_sid};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 任务管理器“启动应用”页的启用 / 禁用状态，按 Run 项的值名记录。
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_VALUE: &str = "BetterPrtSc";

/// [`Autostart`] 的 Windows 实现。
///
/// - [`AutostartMethod::Normal`]：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下名为
///   `BetterPrtSc` 的值，内容是带引号的程序路径。移除时一并删除任务管理器为它记下的启用状态，
///   以后重新开启时不会沿用旧的“已禁用”。
/// - [`AutostartMethod::Elevated`]：根目录下名为 `BetterPrtSc Autostart (<用户 SID>)` 的计划任务，
///   当前用户登录时以最高权限运行本程序，不限运行时长，用电池时照样启动。任务的安全描述符
///   允许当前用户读取和删除它（不提升权限也能查看状态、移除），但不能修改。
pub struct WindowsAutostart {
    run_value: String,
    /// `None` 表示按当前用户的 SID 命名。
    task_name: Option<String>,
    /// `None` 表示当前运行的程序。
    program: Option<PathBuf>,
    /// 计划任务是否以最高权限运行；只有冒烟测试会关掉（以普通权限运行的任务无需管理员权限即可登记）。
    highest: bool,
}

impl Default for WindowsAutostart {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsAutostart {
    pub fn new() -> Self {
        Self {
            run_value: RUN_VALUE.to_owned(),
            task_name: None,
            program: None,
            highest: true,
        }
    }

    /// 冒烟测试用：自定义登记的名称和程序路径，`highest` 为 `false` 时计划任务以普通权限运行。
    #[doc(hidden)]
    pub fn for_smoke_test(
        run_value: &str,
        task_name: &str,
        program: impl Into<PathBuf>,
        highest: bool,
    ) -> Self {
        Self {
            run_value: run_value.to_owned(),
            task_name: Some(task_name.to_owned()),
            program: Some(program.into()),
            highest,
        }
    }

    fn program(&self) -> Result<PathBuf, PlatformError> {
        match &self.program {
            Some(program) => Ok(program.clone()),
            None => current_program(),
        }
    }

    fn task_name(&self, user_sid: &str) -> String {
        self.task_name
            .clone()
            .unwrap_or_else(|| format!("BetterPrtSc Autostart ({user_sid})"))
    }
}

impl Autostart for WindowsAutostart {
    fn entries(&self) -> Result<Vec<AutostartEntry>, PlatformError> {
        let program = self.program()?;
        let mut entries = Vec::new();
        if let Some(command) = registry::read_string(HKEY_CURRENT_USER, RUN_KEY, &self.run_value)? {
            entries.push(AutostartEntry {
                method: AutostartMethod::Normal,
                this_program: same_program(&program_in_command(&command), &program),
            });
        }
        let task_name = self.task_name(&current_user_sid()?);
        let this_program = match TaskRoot::connect()?.task_program(&task_name)? {
            TaskProgram::Missing => None,
            TaskProgram::Found(path) => Some(same_program(&path, &program)),
            // 名字里带着当前用户的 SID，只会是 BetterPrtSc 为这个用户登记的；读不到内容时
            // 按本程序处理，移除时会明确报告缺少权限。
            TaskProgram::Unreadable => Some(true),
        };
        if let Some(this_program) = this_program {
            entries.push(AutostartEntry {
                method: AutostartMethod::Elevated,
                this_program,
            });
        }
        Ok(entries)
    }

    fn enable(&mut self, method: AutostartMethod) -> Result<(), PlatformError> {
        let program = self.program()?;
        match method {
            AutostartMethod::Normal => registry::write_string(
                HKEY_CURRENT_USER,
                RUN_KEY,
                &self.run_value,
                &format!("\"{}\"", program.display()),
            ),
            AutostartMethod::Elevated => {
                let user_sid = current_user_sid()?;
                TaskRoot::connect()?.register(
                    &self.task_name(&user_sid),
                    &task_xml(&program, &user_sid, self.highest),
                    &format!("D:(A;;FA;;;BA)(A;;FA;;;SY)(A;;FRSD;;;{user_sid})"),
                )
            }
        }
    }

    fn disable(&mut self, method: AutostartMethod) -> Result<(), PlatformError> {
        match method {
            AutostartMethod::Normal => {
                registry::delete_value(HKEY_CURRENT_USER, RUN_KEY, &self.run_value)?;
                registry::delete_value(HKEY_CURRENT_USER, STARTUP_APPROVED_KEY, &self.run_value)
            }
            AutostartMethod::Elevated => {
                TaskRoot::connect()?.delete(&self.task_name(&current_user_sid()?))
            }
        }
    }
}

/// 命令行里的程序路径：带引号时取引号内的部分，否则取到 `.exe` 为止（没有就取整行）。
fn program_in_command(command: &str) -> PathBuf {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return PathBuf::from(rest.split('"').next().unwrap_or(rest));
    }
    let end = command
        .to_ascii_lowercase()
        .find(".exe")
        .map_or(command.len(), |i| i + 4);
    PathBuf::from(&command[..end])
}

/// Windows 路径不区分大小写。
fn same_program(a: &Path, b: &Path) -> bool {
    let normalize = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    normalize(a) == normalize(b)
}

/// 登录时启动 `program` 的计划任务定义（Task Scheduler 1.2 架构）。
fn task_xml(program: &Path, user_sid: &str, highest: bool) -> String {
    let command = xml_escape(&format!("\"{}\"", program.display()));
    let directory = xml_escape(
        &program
            .parent()
            .unwrap_or(Path::new(""))
            .display()
            .to_string(),
    );
    let run_level = if highest {
        "HighestAvailable"
    } else {
        "LeastPrivilege"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>BetterPrtSc</Author>
    <Description>登录时以管理员身份启动 BetterPrtSc，不弹出 UAC 提示。由 BetterPrtSc 的“开机自启”和“以管理员身份运行”设置管理。</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user_sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user_sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>{run_level}</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <WorkingDirectory>{directory}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 计划任务里登记的程序。
enum TaskProgram {
    Missing,
    Found(PathBuf),
    /// 任务存在，但没有权限读取它的内容。
    Unreadable,
}

/// 连接到本机任务计划程序的根目录。
struct TaskRoot {
    // 字段按声明顺序释放：COM 接口要先于 COM 库的反初始化释放。
    root: ITaskFolder,
    _com: ComApartment,
}

impl TaskRoot {
    fn connect() -> Result<Self, PlatformError> {
        let com = ComApartment::enter()?;
        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| win_error("连接任务计划程序失败", e))?;
        let empty = VARIANT::default();
        unsafe { service.Connect(&empty, &empty, &empty, &empty) }
            .map_err(|e| win_error("连接任务计划程序失败", e))?;
        let root = unsafe { service.GetFolder(&BSTR::from("\\")) }
            .map_err(|e| win_error("打开任务计划程序根目录失败", e))?;
        Ok(Self { root, _com: com })
    }

    fn task_program(&self, name: &str) -> Result<TaskProgram, PlatformError> {
        let context = |e| win_error(&format!("读取计划任务“{name}”失败"), e);
        let task = match unsafe { self.root.GetTask(&BSTR::from(name)) } {
            Ok(task) => task,
            Err(e) if is_not_found(e.code()) => return Ok(TaskProgram::Missing),
            Err(e) if e.code() == E_ACCESSDENIED => return Ok(TaskProgram::Unreadable),
            Err(e) => return Err(context(e)),
        };
        let actions = unsafe { task.Definition().and_then(|d| d.Actions()) }.map_err(context)?;
        let mut count = 0;
        unsafe { actions.Count(&mut count) }.map_err(context)?;
        for index in 1..=count {
            let action = unsafe { actions.get_Item(index) }.map_err(context)?;
            if let Ok(exec) = action.cast::<IExecAction>() {
                let mut path = BSTR::new();
                unsafe { exec.Path(&mut path) }.map_err(context)?;
                return Ok(TaskProgram::Found(program_in_command(&path.to_string())));
            }
        }
        Ok(TaskProgram::Found(PathBuf::new()))
    }

    /// 登记（或覆盖）计划任务，并设置安全描述符 `sddl`。
    ///
    /// 任务计划程序会拒绝调用方自己写不了的安全描述符（实测普通权限下如此）；被拒绝时
    /// 退回默认的安全描述符再登记一次，此时普通权限的进程可能读不到、删不掉这个任务。
    fn register(&self, name: &str, xml: &str, sddl: &str) -> Result<(), PlatformError> {
        let register = |sddl: &VARIANT| {
            let empty = VARIANT::default();
            unsafe {
                self.root.RegisterTask(
                    &BSTR::from(name),
                    &BSTR::from(xml),
                    TASK_CREATE_OR_UPDATE.0,
                    &empty,
                    &empty,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    sddl,
                )
            }
            .map(drop)
        };
        register(&VARIANT::from(sddl))
            .or_else(|e| {
                if e.code() == E_ACCESSDENIED {
                    register(&VARIANT::default())
                } else {
                    Err(e)
                }
            })
            .map_err(|e| denied_or(e, &format!("登记计划任务“{name}”失败")))
    }

    fn delete(&self, name: &str) -> Result<(), PlatformError> {
        match unsafe { self.root.DeleteTask(&BSTR::from(name), 0) } {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(e.code()) => Ok(()),
            Err(e) => Err(denied_or(e, &format!("删除计划任务“{name}”失败"))),
        }
    }
}

fn is_not_found(code: HRESULT) -> bool {
    code == ERROR_FILE_NOT_FOUND.to_hresult()
}

fn denied_or(error: windows::core::Error, context: &str) -> PlatformError {
    if error.code() == E_ACCESSDENIED {
        PlatformError::new(format!("{context}：需要管理员权限"))
    } else {
        win_error(context, error)
    }
}

/// 在当前线程进入 COM 单线程套间；线程已经以其他模式初始化过 COM 时直接沿用。
struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    fn enter() -> Result<Self, PlatformError> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self { initialized: false });
        }
        result.ok().map_err(|e| win_error("初始化 COM 失败", e))?;
        Ok(Self { initialized: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}
