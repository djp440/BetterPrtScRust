//! 设置界面读写配置文件。
//!
//! 写回前确认配置文件在打开设置界面后没被别人改过（例如用户同时在编辑器里改了它），
//! 改过就不覆盖；写入先写临时文件再改名替换，常驻实例不会读到写了一半的文件。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bps_core::{ConfigError, DEFAULT_CONFIG, SettingsDocument, Texts};

/// 读入的配置文件。
pub(crate) enum Loaded {
    /// 可以编辑。`on_disk` 是读入时文件的全文；文件还不存在时为 `None`，
    /// 此时编辑默认配置，保存时创建文件。
    Editable {
        document: Box<SettingsDocument>,
        on_disk: Option<String>,
    },
    /// 无法在设置界面上编辑。
    Broken(LoadProblem),
}

/// 配置文件无法编辑的原因。
#[derive(Debug)]
pub(crate) enum LoadProblem {
    Invalid(ConfigError),
    NotUtf8,
    Unreadable(io::Error),
}

impl LoadProblem {
    pub(crate) fn localized(&self, texts: &Texts) -> String {
        match self {
            LoadProblem::Invalid(error) => error.localized(texts),
            LoadProblem::NotUtf8 => texts.get("settings-config-not-utf8"),
            LoadProblem::Unreadable(error) => {
                texts.format("settings-config-unreadable", &[("error", error)])
            }
        }
    }
}

/// 无法保存的原因。
#[derive(Debug)]
pub(crate) enum SaveError {
    /// 读入之后配置文件被别人改过（或删除、创建了）。
    ChangedOnDisk,
    Io(io::Error),
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        SaveError::Io(error)
    }
}

pub(crate) fn load(path: &Path) -> Loaded {
    let on_disk = match read(path) {
        Ok(text) => text,
        Err(ReadError::NotUtf8) => return Loaded::Broken(LoadProblem::NotUtf8),
        Err(ReadError::Io(e)) => return Loaded::Broken(LoadProblem::Unreadable(e)),
    };
    let text = on_disk.as_deref().unwrap_or(DEFAULT_CONFIG);
    match SettingsDocument::parse(text) {
        Ok(document) => Loaded::Editable {
            document: Box::new(document),
            on_disk,
        },
        Err(error) => Loaded::Broken(LoadProblem::Invalid(error)),
    }
}

/// 把 `text` 写入 `path`；`path` 当前的内容必须仍是 `on_disk`（`None` 表示文件不存在）。
pub(crate) fn save(path: &Path, on_disk: Option<&str>, text: &str) -> Result<(), SaveError> {
    let current = match read(path) {
        Ok(current) => current,
        Err(ReadError::NotUtf8) => return Err(SaveError::ChangedOnDisk),
        Err(ReadError::Io(e)) => return Err(SaveError::Io(e)),
    };
    if current.as_deref() != on_disk {
        return Err(SaveError::ChangedOnDisk);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = temporary_path(path);
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })?;
    Ok(())
}

enum ReadError {
    NotUtf8,
    Io(io::Error),
}

/// 读取全文；文件不存在时为 `Ok(None)`。
fn read(path: &Path) -> Result<Option<String>, ReadError> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| ReadError::NotUtf8),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ReadError::Io(e)),
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".saving");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用的临时目录，测试结束时删除。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("bps-ui-store-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn editable(loaded: Loaded) -> (SettingsDocument, Option<String>) {
        match loaded {
            Loaded::Editable { document, on_disk } => (*document, on_disk),
            Loaded::Broken(_) => panic!("配置文件应当可以编辑"),
        }
    }

    #[test]
    fn a_missing_config_file_is_edited_as_the_default_and_created_on_save() {
        let dir = TempDir::new("missing");
        let path = dir.0.join("BetterPrtSc").join("config.toml");

        let (document, on_disk) = editable(load(&path));
        assert_eq!(on_disk, None);
        assert_eq!(document.text(), DEFAULT_CONFIG);

        let mut settings = document.settings().clone();
        settings.check_for_updates = true;
        let text = document.render(&settings).unwrap();
        save(&path, on_disk.as_deref(), &text).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(editable(load(&path)).0.settings().check_for_updates);
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "只应留下 config.toml");
    }

    #[test]
    fn saving_does_not_overwrite_changes_made_elsewhere_since_loading() {
        let dir = TempDir::new("conflict");
        let path = dir.0.join("config.toml");
        fs::write(&path, DEFAULT_CONFIG).unwrap();
        let (document, on_disk) = editable(load(&path));

        let edited_elsewhere = DEFAULT_CONFIG.replace("PrintScreen", "F9");
        fs::write(&path, &edited_elsewhere).unwrap();
        let mut settings = document.settings().clone();
        settings.autostart = true;
        let text = document.render(&settings).unwrap();

        assert!(matches!(
            save(&path, on_disk.as_deref(), &text),
            Err(SaveError::ChangedOnDisk)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), edited_elsewhere);
    }

    #[test]
    fn a_broken_config_file_cannot_be_edited() {
        let dir = TempDir::new("broken");
        let path = dir.0.join("config.toml");
        fs::write(&path, "language = \"fr\"\n").unwrap();

        assert!(matches!(
            load(&path),
            Loaded::Broken(LoadProblem::Invalid(_))
        ));
    }
}
