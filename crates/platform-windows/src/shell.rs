//! 给界面子进程用的外壳操作：用默认程序打开文件或网址、选择文件夹、
//! 判断某个位置是否能被普通权限的程序改写、把前台权限让给其他进程。

use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use bps_platform::PlatformError;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    AccessCheck, DACL_SECURITY_INFORMATION, DuplicateToken, GENERIC_MAPPING,
    GROUP_SECURITY_INFORMATION, GetFileSecurityW, GetTokenInformation, MapGenericMask,
    OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, PSECURITY_DESCRIPTOR, SecurityImpersonation,
    TOKEN_DUPLICATE, TOKEN_ELEVATION_TYPE, TOKEN_LINKED_TOKEN, TOKEN_QUERY, TokenElevationType,
    TokenElevationTypeFull, TokenLinkedToken,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_DATA,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::wide_path;

/// 用默认程序打开文件或网址，或在资源管理器中打开文件夹。
pub fn open_with_default_program(target: &OsStr) -> Result<(), PlatformError> {
    let wide: Vec<u16> = target.encode_wide().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 返回值大于 32 表示成功。
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(PlatformError::new(format!(
            "无法打开 {}（ShellExecute 返回 {}）",
            Path::new(target).display(),
            result.0 as usize
        )))
    }
}

/// 弹出系统的“选择文件夹”对话框，从 `initial` 开始浏览；取消或失败时返回 `None`。
///
/// 对话框有自己的消息循环，会阻塞到用户关闭它，应在界面线程以外的线程调用。
pub fn pick_folder(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let picked = pick_folder_with_com(title, initial);
        if initialized.is_ok() {
            CoUninitialize();
        }
        picked
    }
}

unsafe fn pick_folder_with_com(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .ok()?;
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Some(dir) = initial.filter(|dir| dir.is_dir())
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&wide_path(dir), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // 用户取消时返回错误。
        dialog.Show(None).ok()?;
        let name = dialog
            .GetResult()
            .ok()?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .ok()?;
        let path = PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        Some(path)
    }
}

/// 普通权限（未提升）的程序能否写入 `path`（文件：改写内容；目录：在其中新建文件）。
///
/// 以管理员身份运行时按它对应的受限令牌判断。无法判断时返回 `None`。
pub fn standard_user_can_write(path: &Path) -> Option<bool> {
    unsafe {
        let token = Handle::process_token()?;
        let standard = token.standard_user_token()?;
        let mut impersonation = HANDLE::default();
        DuplicateToken(standard.0, SecurityImpersonation, &mut impersonation).ok()?;
        let impersonation = Handle(impersonation);

        let wide = wide_path(path);
        let info =
            (OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION).0;
        let mut needed = 0u32;
        let _ = GetFileSecurityW(&wide, info, None, 0, &mut needed);
        if needed == 0 {
            return None;
        }
        // 安全描述符要求按指针对齐。
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        let descriptor = PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast());
        if !GetFileSecurityW(&wide, info, Some(descriptor), needed, &mut needed).as_bool() {
            return None;
        }

        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ.0,
            GenericWrite: FILE_GENERIC_WRITE.0,
            GenericExecute: FILE_GENERIC_EXECUTE.0,
            GenericAll: FILE_ALL_ACCESS.0,
        };
        // 对目录而言 FILE_WRITE_DATA 就是 FILE_ADD_FILE。
        let mut desired = FILE_WRITE_DATA.0;
        MapGenericMask(&mut desired, &mapping);
        let mut privileges = PRIVILEGE_SET::default();
        let mut privileges_length = size_of::<PRIVILEGE_SET>() as u32;
        let mut granted = 0u32;
        let mut allowed = windows::core::BOOL(0);
        AccessCheck(
            descriptor,
            impersonation.0,
            desired,
            &mapping,
            Some(&mut privileges),
            &mut privileges_length,
            &mut granted,
            &mut allowed,
        )
        .ok()?;
        Some(allowed.as_bool())
    }
}

/// 允许任何进程把窗口带到前台。再次启动的程序把“打开设置”转交给常驻实例前调用，
/// 常驻实例打开的设置界面才能出现在最前面。
pub fn allow_any_process_to_set_foreground() {
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// 离开作用域时关闭的内核对象句柄。
struct Handle(HANDLE);

impl Handle {
    unsafe fn process_token() -> Option<Handle> {
        let mut token = HANDLE::default();
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut token,
            )
        }
        .ok()?;
        Some(Handle(token))
    }

    /// 已提升的令牌换成它对应的受限令牌；未提升的就是它自己。
    unsafe fn standard_user_token(self) -> Option<Handle> {
        unsafe {
            let mut elevation = TOKEN_ELEVATION_TYPE::default();
            let mut length = 0u32;
            GetTokenInformation(
                self.0,
                TokenElevationType,
                Some(&mut elevation as *mut _ as *mut _),
                size_of::<TOKEN_ELEVATION_TYPE>() as u32,
                &mut length,
            )
            .ok()?;
            if elevation != TokenElevationTypeFull {
                return Some(self);
            }
            let mut linked = TOKEN_LINKED_TOKEN::default();
            GetTokenInformation(
                self.0,
                TokenLinkedToken,
                Some(&mut linked as *mut _ as *mut _),
                size_of::<TOKEN_LINKED_TOKEN>() as u32,
                &mut length,
            )
            .ok()?;
            Some(Handle(linked.LinkedToken))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}
