use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bps_platform::{FileSystem, NewFile, PlatformError};

/// 假“图片”文件夹的默认位置。
pub const DEFAULT_PICTURES_DIR: &str = r"C:\Users\Fake\Pictures";

/// 内存中的目录树：只记录目录和文件内容，行为尽量与真实文件系统一致
/// （写文件要求上级目录已存在、同名文件不覆盖）。
pub(crate) struct FsState {
    pub pictures_dir: PathBuf,
    dirs: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
    /// 这些目录及其子孙不可写。
    read_only: Vec<PathBuf>,
}

impl Default for FsState {
    fn default() -> Self {
        let mut state = Self {
            pictures_dir: PathBuf::from(DEFAULT_PICTURES_DIR),
            dirs: BTreeSet::new(),
            files: BTreeMap::new(),
            read_only: Vec::new(),
        };
        state.add_dir(&PathBuf::from(DEFAULT_PICTURES_DIR));
        state
    }
}

impl FsState {
    /// 创建目录及所有上级目录（不检查是否可写）。
    pub fn add_dir(&mut self, path: &Path) {
        for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
            self.dirs.insert(ancestor.to_path_buf());
        }
    }

    /// 放入一个文件，自动创建上级目录（不检查是否可写）。
    pub fn add_file(&mut self, path: &Path, contents: Vec<u8>) {
        if let Some(parent) = path.parent() {
            self.add_dir(parent);
        }
        self.files.insert(path.to_path_buf(), contents);
    }

    pub fn add_read_only_dir(&mut self, path: &Path) {
        self.add_dir(path);
        self.read_only.push(path.to_path_buf());
    }

    pub fn file(&self, path: &Path) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }

    pub fn file_paths(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    fn check_writable(&self, path: &Path) -> Result<(), PlatformError> {
        match self.read_only.iter().find(|dir| path.starts_with(dir)) {
            Some(dir) => Err(PlatformError::new(format!(
                "拒绝访问：{} 是只读目录",
                dir.display()
            ))),
            None => Ok(()),
        }
    }

    /// 写入文件：不存在时新建，存在时覆盖（假编码器写视频用）。上级目录必须已存在且可写。
    pub fn overwrite(&mut self, path: &Path, contents: Vec<u8>) -> Result<(), PlatformError> {
        if self.dirs.contains(path) {
            return Err(PlatformError::new(format!("{} 是目录", path.display())));
        }
        self.check_parent(path)?;
        self.check_writable(path)?;
        self.files.insert(path.to_path_buf(), contents);
        Ok(())
    }

    fn check_parent(&self, path: &Path) -> Result<(), PlatformError> {
        let parent = path.parent().unwrap_or(Path::new(""));
        if self.dirs.contains(parent) {
            Ok(())
        } else {
            Err(PlatformError::new(format!(
                "系统找不到指定的路径：{}",
                parent.display()
            )))
        }
    }
}

pub(crate) struct FakeFileSystem(pub Arc<Mutex<FsState>>);

impl FileSystem for FakeFileSystem {
    fn pictures_dir(&self) -> Result<PathBuf, PlatformError> {
        Ok(self.0.lock().unwrap().pictures_dir.clone())
    }

    fn create_dir_all(&mut self, path: &Path) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        if let Some(file) = path.ancestors().find(|p| state.files.contains_key(*p)) {
            return Err(PlatformError::new(format!(
                "无法创建目录：{} 是文件",
                file.display()
            )));
        }
        if !state.dirs.contains(path) {
            state.check_writable(path)?;
            state.add_dir(path);
        }
        Ok(())
    }

    fn create_new(&mut self, path: &Path, contents: &[u8]) -> Result<NewFile, PlatformError> {
        let mut state = self.0.lock().unwrap();
        if state.files.contains_key(path) || state.dirs.contains(path) {
            return Ok(NewFile::AlreadyExists);
        }
        let parent = path.parent().unwrap_or(Path::new(""));
        if !state.dirs.contains(parent) {
            return Err(PlatformError::new(format!(
                "系统找不到指定的路径：{}",
                parent.display()
            )));
        }
        state.check_writable(path)?;
        state.files.insert(path.to_path_buf(), contents.to_vec());
        Ok(NewFile::Created)
    }

    fn rename_new(&mut self, from: &Path, to: &Path) -> Result<NewFile, PlatformError> {
        let mut state = self.0.lock().unwrap();
        if !state.files.contains_key(from) {
            return Err(PlatformError::new(format!(
                "系统找不到指定的文件：{}",
                from.display()
            )));
        }
        if state.files.contains_key(to) || state.dirs.contains(to) {
            return Ok(NewFile::AlreadyExists);
        }
        state.check_parent(to)?;
        state.check_writable(from)?;
        state.check_writable(to)?;
        let contents = state.files.remove(from).expect("刚刚检查过");
        state.files.insert(to.to_path_buf(), contents);
        Ok(NewFile::Created)
    }

    fn remove_file(&mut self, path: &Path) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        if !state.files.contains_key(path) {
            return Err(PlatformError::new(format!(
                "系统找不到指定的文件：{}",
                path.display()
            )));
        }
        state.check_writable(path)?;
        state.files.remove(path);
        Ok(())
    }
}
