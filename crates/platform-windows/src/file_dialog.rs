//! 文件对话框：系统的“另存为”对话框（`IFileSaveDialog`）。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use bps_platform::{FileDialog, PlatformError, SaveAsOptions};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog, IFileSaveDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, PCWSTR};

use crate::autostart::ComApartment;
use crate::error::{wide_path, win_error};
use crate::overlay::visible_overlay_window;

/// 系统文件对话框。必须在事件循环的线程（选区遮罩所在的线程）上使用。
///
/// 对话框是模态的：它自己的消息循环照常处理热键等消息（事件进队列，等对话框关闭后再处理），
/// 选区遮罩显示着时以遮罩为所有者窗口，所以显示在遮罩之上，且遮罩在此期间不接收输入。
#[derive(Debug, Default)]
pub struct WindowsFileDialog;

impl WindowsFileDialog {
    pub fn new() -> Self {
        Self
    }
}

impl FileDialog for WindowsFileDialog {
    fn save_as(&mut self, options: &SaveAsOptions) -> Result<Option<PathBuf>, PlatformError> {
        let _com = ComApartment::enter()?;
        unsafe {
            let dialog: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| win_error("打开“另存为”对话框失败", e))?;
            // 过滤器里的字符串要活到对话框关闭。
            let names: Vec<HSTRING> = options
                .file_types
                .iter()
                .map(|t| HSTRING::from(t.name.as_str()))
                .collect();
            let specs: Vec<HSTRING> = options
                .file_types
                .iter()
                .map(|t| {
                    let patterns: Vec<String> =
                        t.extensions.iter().map(|e| format!("*.{e}")).collect();
                    HSTRING::from(patterns.join(";"))
                })
                .collect();
            let filters: Vec<COMDLG_FILTERSPEC> = names
                .iter()
                .zip(&specs)
                .map(|(name, spec)| COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name.as_ptr()),
                    pszSpec: PCWSTR(spec.as_ptr()),
                })
                .collect();
            let setup = || -> windows::core::Result<()> {
                if !filters.is_empty() {
                    dialog.SetFileTypes(&filters)?;
                    // 下标从 1 开始。
                    dialog.SetFileTypeIndex(options.default_file_type as u32 + 1)?;
                }
                // 设了默认扩展名，用户切换文件类型时对话框会跟着改文件名的扩展名。
                if let Some(extension) = options
                    .file_types
                    .get(options.default_file_type)
                    .and_then(|t| t.extensions.first())
                {
                    dialog.SetDefaultExtension(&HSTRING::from(extension.as_str()))?;
                }
                dialog.SetFileName(&HSTRING::from(options.file_name.as_str()))?;
                let flags = dialog.GetOptions()?;
                dialog.SetOptions(
                    flags | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
                )?;
                Ok(())
            };
            setup().map_err(|e| win_error("设置“另存为”对话框失败", e))?;
            if let Some(folder) = options.folder.as_deref().and_then(existing_folder) {
                // 只在系统没有记住上一次的位置时生效；目录打不开就用系统默认位置。
                let _ = dialog.SetDefaultFolder(&folder);
            }
            match dialog.Show(visible_overlay_window()) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
                Err(e) => return Err(win_error("“另存为”对话框出错", e)),
            }
            let item = dialog
                .GetResult()
                .map_err(|e| win_error("读取另存为的位置失败", e))?;
            let name = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|e| win_error("读取另存为的位置失败", e))?;
            let path = PathBuf::from(OsString::from_wide(name.as_wide()));
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(path))
        }
    }
}

/// `folder` 或它最近的一个存在的上级目录。
fn existing_folder(folder: &Path) -> Option<IShellItem> {
    folder
        .ancestors()
        .filter(|p| p.is_dir())
        .find_map(|p| unsafe { SHCreateItemFromParsingName(&wide_path(p), None).ok() })
}
