//! 注册表的小工具：读写字符串与 DWORD 值、删除值、监视键的变化。

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use bps_platform::PlatformError;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_THREAD_AGNOSTIC, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegDeleteKeyValueW, RegGetValueW,
    RegNotifyChangeKeyValue, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::HSTRING;

use crate::error::win_error;

fn check(context: impl FnOnce() -> String, status: WIN32_ERROR) -> Result<(), PlatformError> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(win_error(&context(), status.to_hresult().into()))
    }
}

/// 读取字符串值（`REG_EXPAND_SZ` 会被展开）；键或值不存在时返回 `None`。
pub(crate) fn read_string(
    root: HKEY,
    subkey: &str,
    value: &str,
) -> Result<Option<String>, PlatformError> {
    let (subkey_w, value_w) = (HSTRING::from(subkey), HSTRING::from(value));
    let context = || format!("读取注册表 {subkey}\\{value} 失败");
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &subkey_w,
            &value_w,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(context, status)?;
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &subkey_w,
            &value_w,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(context, status)?;
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..len])))
}

/// 读取 DWORD 值；键或值不存在时返回 `None`。
pub(crate) fn read_dword(
    root: HKEY,
    subkey: &str,
    value: &str,
) -> Result<Option<u32>, PlatformError> {
    let mut data = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut bytes),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(|| format!("读取注册表 {subkey}\\{value} 失败"), status)?;
    Ok(Some(data))
}

/// 写入字符串值（`REG_SZ`），必要时创建键。
pub(crate) fn write_string(
    root: HKEY,
    subkey: &str,
    value: &str,
    data: &str,
) -> Result<(), PlatformError> {
    let wide: Vec<u16> = data.encode_utf16().chain([0]).collect();
    let status = unsafe {
        RegSetKeyValueW(
            root,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            (wide.len() * 2) as u32,
        )
    };
    check(|| format!("写入注册表 {subkey}\\{value} 失败"), status)
}

/// 删除一个值；键或值本来就不存在时什么也不做。
pub(crate) fn delete_value(root: HKEY, subkey: &str, value: &str) -> Result<(), PlatformError> {
    let status = unsafe { RegDeleteKeyValueW(root, &HSTRING::from(subkey), &HSTRING::from(value)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(|| format!("删除注册表 {subkey}\\{value} 失败"), status)
}

/// 监视一个注册表键里的值被修改：有变化时 [`handle`](Self::handle) 变为有信号，
/// 处理后调用 [`rearm`](Self::rearm) 继续监视。被丢弃时停止监视。
pub(crate) struct RegistryWatch {
    key: HKEY,
    event: OwnedHandle,
}

impl RegistryWatch {
    pub fn new(root: HKEY, subkey: &str) -> Result<Self, PlatformError> {
        let mut key = HKEY::default();
        let status =
            unsafe { RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_NOTIFY, &mut key) };
        check(|| format!("打开注册表 {subkey} 失败"), status)?;
        let event = match unsafe { CreateEventW(None, false, false, None) } {
            Ok(event) => unsafe { OwnedHandle::from_raw_handle(event.0) },
            Err(e) => {
                let _ = unsafe { RegCloseKey(key) };
                return Err(win_error("创建事件失败", e));
            }
        };
        let watch = Self { key, event };
        watch.rearm()?;
        Ok(watch)
    }

    /// 有变化时变为有信号的事件（自动重置）。
    pub fn handle(&self) -> HANDLE {
        HANDLE(self.event.as_raw_handle())
    }

    /// 布防下一次通知；每次通知后都要重新调用。
    pub fn rearm(&self) -> Result<(), PlatformError> {
        let status = unsafe {
            RegNotifyChangeKeyValue(
                self.key,
                false,
                REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_THREAD_AGNOSTIC,
                Some(self.handle()),
                true,
            )
        };
        check(|| "监视注册表失败".to_owned(), status)
    }
}

impl Drop for RegistryWatch {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.key) };
    }
}
