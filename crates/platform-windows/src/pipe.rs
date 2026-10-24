//! 常驻实例的命名管道：既是单实例的“锁”，也是其他进程转交请求的通道。
//!
//! - 常驻实例一侧：[`ResidentPipe::claim`] 以 `FILE_FLAG_FIRST_PIPE_INSTANCE` 创建第一个
//!   管道实例，失败说明已有常驻实例；[`ResidentPipe::serve`] 在后台线程接收请求，
//!   以 [`PlatformEvent::RequestReceived`] 投递给事件循环，并把回复写回管道。
//!   监听线程总是先建好下一个管道实例再处理当前连接，管道因此始终存在，单实例不会被打破。
//! - CLI 一侧：[`PipeChannel`] 实现 [`ResidentChannel`]，用 `CallNamedPipeW` 一次完成
//!   连接、发送、等待回复；管道不存在时拉起 `BetterPrtSc.exe`。
//!
//! 管道只搬运文本，请求和回复的含义由 `core` 决定。
//!
//! 管道的访问权限显式授予本机系统、管理员组和当前用户：以管理员身份运行的常驻实例若用默认
//! 权限创建管道，普通权限的 `bps` 和再次启动的程序都打不开它。

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use bps_platform::{ChannelError, IncomingRequest, PlatformError, PlatformEvent, ResidentChannel};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, ERROR_SEM_TIMEOUT,
    GetLastError, HANDLE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
    ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, DETACHED_PROCESS, GetCurrentProcessId,
};
use windows::core::{HRESULT, PCWSTR};

use crate::EventSender;
use crate::error::win_error;
use crate::token::current_user_sid;

/// 一条请求的最大字节数；请求只有一行命令，远小于这个值。
const MAX_REQUEST_BYTES: usize = 4096;
/// 一条回复的最大字节数。
const MAX_REPLY_BYTES: usize = 64 * 1024;
/// 管道实例都忙时，CLI 等待空闲实例的时间。
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// 拉起常驻实例后等它创建好管道的时间。
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);

/// 当前登录会话里常驻实例的管道名。按会话区分：快速用户切换时每个会话各有一个常驻实例。
pub fn resident_pipe_name() -> String {
    let mut session = 0;
    // 失败时退回会话 0；只影响多会话场景。
    let _ = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    format!(r"\\.\pipe\BetterPrtSc-{session}")
}

/// 抢占常驻实例管道的结果。
pub enum PipeClaim {
    /// 本进程占用了管道，应成为常驻实例。
    Claimed(ResidentPipe),
    /// 管道已被占用：已有常驻实例在运行。
    AlreadyRunning,
}

/// 本进程作为常驻实例占用的命名管道。
pub struct ResidentPipe {
    name: Vec<u16>,
    first_instance: OwnedHandle,
}

impl ResidentPipe {
    /// 尝试以第一个实例的身份创建管道 `name`。
    pub fn claim(name: &str) -> Result<PipeClaim, PlatformError> {
        let name = wide(name);
        match create_instance(&name, true) {
            Ok(handle) => Ok(PipeClaim::Claimed(ResidentPipe {
                name,
                first_instance: handle,
            })),
            Err(e) if e == ERROR_ACCESS_DENIED.0 || e == ERROR_PIPE_BUSY.0 => {
                Ok(PipeClaim::AlreadyRunning)
            }
            Err(e) => Err(PlatformError::new(format!(
                "创建命名管道失败：{}",
                HRESULT::from_win32(e).message()
            ))),
        }
    }

    /// 在后台线程接收请求，投递给 `events` 所属的事件循环。监听线程随进程结束。
    pub fn serve(self, events: EventSender) -> Result<(), PlatformError> {
        thread::Builder::new()
            .name("bps-resident-pipe".to_owned())
            .spawn(move || listen(self, events))
            .map(drop)
            .map_err(|e| PlatformError::new(format!("启动命名管道线程失败：{e}")))
    }
}

/// 监听循环：等待连接 → 先建好下一个实例 → 在单独的线程里处理这个连接。
fn listen(pipe: ResidentPipe, events: EventSender) {
    let ResidentPipe {
        name,
        first_instance: mut listening,
    } = pipe;
    loop {
        if let Err(e) = unsafe { ConnectNamedPipe(raw(&listening), None) }
            && e.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0)
        {
            // 客户端刚连上就断开等：重置这个实例继续等待；稍作停顿，避免持续出错时空转。
            let _ = unsafe { DisconnectNamedPipe(raw(&listening)) };
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        match create_instance(&name, false) {
            Ok(next) => {
                let connection = std::mem::replace(&mut listening, next);
                let events = events.clone();
                // 线程创建失败时连接随闭包一起关闭，客户端会收到“没有回复”。
                let _ = thread::Builder::new()
                    .name("bps-pipe-connection".to_owned())
                    .spawn(move || serve_connection(&connection, &events));
            }
            Err(_) => {
                // 建不了新实例时就地处理，处理完复用当前实例，保证管道始终存在。
                serve_connection(&listening, &events);
            }
        }
    }
}

/// 处理一个已连接的客户端：读一条请求，交给事件循环，等回复写回，然后断开。
fn serve_connection(connection: &OwnedHandle, events: &EventSender) {
    let handle = raw(connection);
    if let Some(message) = read_request(handle) {
        let (reply_tx, reply_rx) = mpsc::sync_channel::<String>(1);
        let request = IncomingRequest::new(message, move |reply| {
            let _ = reply_tx.send(reply);
        });
        // 事件循环已结束或请求被丢弃而没有回复时，直接断开，客户端会收到“没有回复”。
        if events.send(PlatformEvent::RequestReceived(request)).is_ok()
            && let Ok(reply) = reply_rx.recv()
        {
            let _ = unsafe { WriteFile(handle, Some(reply.as_bytes()), None, None) };
            // 等客户端读完再断开，否则回复会被丢弃。
            let _ = unsafe { FlushFileBuffers(handle) };
        }
    }
    let _ = unsafe { DisconnectNamedPipe(handle) };
}

/// 读一条完整的请求消息；读取失败或超长时返回 `None`。
fn read_request(handle: HANDLE) -> Option<String> {
    let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
    let mut read = 0u32;
    unsafe { ReadFile(handle, Some(&mut buffer), Some(&mut read), None) }.ok()?;
    buffer.truncate(read as usize);
    // 非 UTF-8 的内容照样交给 core，由它回复“无法识别的请求”。
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// 创建一个消息模式的管道实例；失败时返回 Win32 错误码。
fn create_instance(name: &[u16], first: bool) -> Result<OwnedHandle, u32> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let attributes = pipe_security_descriptor().map(|descriptor| SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    });
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(open_mode.0),
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_REPLY_BYTES as u32,
            MAX_REQUEST_BYTES as u32,
            0,
            attributes.as_ref().map(|a| a as *const SECURITY_ATTRIBUTES),
        )
    };
    if handle.is_invalid() {
        return Err(unsafe { GetLastError() }.0);
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

/// 管道的安全描述符：本机系统、管理员组和当前用户可以完全访问。进程内只构造一次、不释放。
/// 构造失败（取不到用户 SID 等）时返回 `None`，退回系统默认的权限。
fn pipe_security_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
    // 指针不是 Send/Sync，按整数缓存。
    static DESCRIPTOR: OnceLock<Option<usize>> = OnceLock::new();
    let address = DESCRIPTOR.get_or_init(|| {
        let sid = current_user_sid().ok()?;
        let sddl = wide(&format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})"));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .ok()?;
        Some(descriptor.0 as usize)
    });
    address.map(|a| PSECURITY_DESCRIPTOR(a as *mut _))
}

/// CLI 一侧通往常驻实例的命名管道通道。
pub struct PipeChannel {
    name: Vec<u16>,
    resident_exe: PathBuf,
}

impl PipeChannel {
    /// `name` 是常驻实例的管道名（通常是 [`resident_pipe_name`]），
    /// `resident_exe` 是常驻实例没在运行时要拉起的 `BetterPrtSc.exe`。
    pub fn new(name: &str, resident_exe: impl Into<PathBuf>) -> Self {
        Self {
            name: wide(name),
            resident_exe: resident_exe.into(),
        }
    }

    /// 管道是否存在（不占用管道实例）。
    fn pipe_exists(&self) -> bool {
        // 等待 1 ms：0 表示“使用服务端默认超时”。
        let available = unsafe { WaitNamedPipeW(PCWSTR(self.name.as_ptr()), 1) }.as_bool();
        available || unsafe { GetLastError() } == ERROR_SEM_TIMEOUT
    }

    /// 以独立进程启动常驻实例：不继承控制台和标准输入输出（否则调用 `bps` 的脚本
    /// 会一直等管道关闭），工作目录设为程序目录，尽量脱离调用方的作业对象。
    fn spawn_resident(&self) -> std::io::Result<std::process::Child> {
        let spawn = |flags: u32| {
            let mut command = Command::new(&self.resident_exe);
            if let Some(dir) = self.resident_exe.parent().filter(|d| d.is_dir()) {
                command.current_dir(dir);
            }
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        spawn(DETACHED_PROCESS.0 | CREATE_BREAKAWAY_FROM_JOB.0).or_else(|e| {
            // 调用方所在的作业不允许脱离时退回普通启动。
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) {
                spawn(DETACHED_PROCESS.0)
            } else {
                Err(e)
            }
        })
    }
}

impl ResidentChannel for PipeChannel {
    fn send(&mut self, message: &str) -> Result<String, ChannelError> {
        let mut reply = vec![0u8; MAX_REPLY_BYTES];
        let mut read = 0u32;
        let ok = unsafe {
            CallNamedPipeW(
                PCWSTR(self.name.as_ptr()),
                Some(message.as_ptr().cast()),
                message.len() as u32,
                Some(reply.as_mut_ptr().cast()),
                reply.len() as u32,
                &mut read,
                BUSY_TIMEOUT.as_millis() as u32,
            )
        }
        .as_bool();
        if ok {
            reply.truncate(read as usize);
            return Ok(String::from_utf8_lossy(&reply).into_owned());
        }
        let error = unsafe { GetLastError() };
        let failed = |text: &str| Err(ChannelError::Failed(PlatformError::new(text)));
        match error {
            ERROR_FILE_NOT_FOUND => Err(ChannelError::NotRunning),
            ERROR_SEM_TIMEOUT | ERROR_PIPE_BUSY => failed("常驻实例忙，等待超时"),
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => {
                failed("常驻实例没有回复就断开了连接")
            }
            ERROR_MORE_DATA => failed("常驻实例的回复过长"),
            ERROR_ACCESS_DENIED => failed("无权访问常驻实例（它可能以管理员身份运行）"),
            other => Err(ChannelError::Failed(win_error(
                "转交请求失败",
                HRESULT::from_win32(other.0).into(),
            ))),
        }
    }

    fn launch_resident_instance(&mut self) -> Result<(), PlatformError> {
        let mut child = self.spawn_resident().map_err(|e| {
            PlatformError::new(format!("无法启动 {}：{e}", self.resident_exe.display()))
        })?;
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        loop {
            if self.pipe_exists() {
                return Ok(());
            }
            match child.try_wait() {
                // 退出码 0：多半是另一个常驻实例抢先启动，它已把激活请求转交过去，再试一次即可。
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(PlatformError::new(format!(
                        "常驻实例启动后立即退出（{status}）"
                    )));
                }
                Ok(None) | Err(_) => {}
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::new(format!(
                    "常驻实例在 {} 秒内没有就绪",
                    LAUNCH_TIMEOUT.as_secs()
                )));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// 以 NUL 结尾的 UTF-16 字符串。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
