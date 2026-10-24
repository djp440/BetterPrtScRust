//! 当前进程的访问令牌：是否以管理员权限运行、当前用户的 SID，以及以管理员身份启动本程序。

use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use bps_platform::{Elevation, PlatformError};
use windows::Win32::Foundation::{ERROR_CANCELLED, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TOKEN_USER,
    TokenElevation, TokenUser,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use crate::error::{wide_path, win_error};

/// 以管理员身份重新启动常驻实例时附加的命令行参数：新进程据此等待旧进程让出单实例，
/// 并且在仍然没有管理员权限时不再重试。
pub const RESTARTED_AS_ADMINISTRATOR_ARG: &str = "--restarted-as-administrator";

/// 当前进程是否以管理员权限运行（UAC 提升后的令牌）。查询失败时按“否”处理。
pub fn is_elevated() -> bool {
    let mut elevation = TOKEN_ELEVATION::default();
    query_token(
        TokenElevation,
        (&raw mut elevation).cast(),
        size_of::<TOKEN_ELEVATION>() as u32,
    )
    .is_ok()
        && elevation.TokenIsElevated != 0
}

/// 当前用户的 SID 字符串（如 `S-1-5-21-…`）。以管理员身份运行时与普通权限时相同。
pub(crate) fn current_user_sid() -> Result<String, PlatformError> {
    // TOKEN_USER 后面紧跟着变长的 SID，缓冲区按 u64 对齐分配。
    let mut buffer = vec![0u64; 64];
    query_token(
        TokenUser,
        buffer.as_mut_ptr().cast(),
        (buffer.len() * size_of::<u64>()) as u32,
    )?;
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }
        .map_err(|e| win_error("转换用户 SID 失败", e))?;
    let sid = unsafe { text.to_string() };
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    sid.map_err(|e| PlatformError::new(format!("用户 SID 不是有效的文本：{e}")))
}

fn query_token(
    class: TOKEN_INFORMATION_CLASS,
    buffer: *mut core::ffi::c_void,
    size: u32,
) -> Result<(), PlatformError> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| win_error("打开进程令牌失败", e))?;
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&token)),
            class,
            Some(buffer),
            size,
            &mut returned,
        )
    }
    .map_err(|e| win_error("读取进程令牌失败", e))
}

/// 以管理员身份启动本程序（弹出 UAC 提示），参数为 `args`；返回新进程的句柄。
/// 用户在 UAC 提示里点“否”时返回错误。
fn launch_self_as_administrator(args: &str) -> Result<OwnedHandle, PlatformError> {
    let exe = std::env::current_exe()
        .map_err(|e| PlatformError::new(format!("无法确定程序路径：{e}")))?;
    let file = wide_path(&exe);
    let directory = wide_path(exe.parent().unwrap_or(Path::new(".")));
    let parameters = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            PlatformError::new("用户在 UAC 提示里取消了以管理员身份运行")
        } else {
            win_error("以管理员身份启动失败", e)
        }
    })?;
    if info.hProcess.is_invalid() {
        return Err(PlatformError::new("以管理员身份启动失败：没有得到新进程"));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) })
}

/// 以管理员身份运行本程序（弹出 UAC 提示），等它结束并返回退出码。
pub fn run_self_as_administrator_and_wait(args: &str) -> Result<u32, PlatformError> {
    let process = launch_self_as_administrator(args)?;
    let handle = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&process));
    if unsafe { WaitForSingleObject(handle, INFINITE) } != WAIT_OBJECT_0 {
        return Err(PlatformError::new("等待以管理员身份运行的进程结束失败"));
    }
    let mut code = 0;
    unsafe { GetExitCodeProcess(handle, &mut code) }.map_err(|e| win_error("读取退出码失败", e))?;
    Ok(code)
}

/// [`Elevation`] 的 Windows 实现：令牌是否提升；以管理员身份重新启动 `BetterPrtSc.exe`。
#[derive(Debug, Default)]
pub struct WindowsElevation;

impl WindowsElevation {
    pub fn new() -> Self {
        Self
    }
}

impl Elevation for WindowsElevation {
    fn is_elevated(&self) -> bool {
        is_elevated()
    }

    fn restart_as_administrator(&mut self) -> Result<(), PlatformError> {
        launch_self_as_administrator(RESTARTED_AS_ADMINISTRATOR_ARG).map(drop)
    }
}

/// 当前程序（`BetterPrtSc.exe`）的路径。
pub(crate) fn current_program() -> Result<PathBuf, PlatformError> {
    std::env::current_exe().map_err(|e| PlatformError::new(format!("无法确定程序路径：{e}")))
}
