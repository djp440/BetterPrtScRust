use std::path::{Path, PathBuf};

use crate::PlatformError;

/// [`FileSystem::create_new`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewFile {
    /// 文件已新建并写入。
    Created,
    /// 同名文件已存在，没有做任何改动。
    AlreadyExists,
}

/// 文件系统：自动存储把产物写入磁盘时使用。
pub trait FileSystem {
    /// 当前用户的“图片”文件夹（Windows 上是 `FOLDERID_Pictures`，可能已被用户移动）。
    fn pictures_dir(&self) -> Result<PathBuf, PlatformError>;

    /// 创建目录及所有缺失的上级目录；目录已存在时什么也不做。
    fn create_dir_all(&mut self, path: &Path) -> Result<(), PlatformError>;

    /// 新建文件并写入 `contents`。同名文件已存在时不改动它，返回 [`NewFile::AlreadyExists`]，
    /// 由调用方换一个文件名再试；检查与创建是原子的，不会覆盖并发出现的同名文件。
    fn create_new(&mut self, path: &Path, contents: &[u8]) -> Result<NewFile, PlatformError>;

    /// 把文件 `from` 改名（移动）为 `to`。`to` 已存在时不改动任何文件，返回
    /// [`NewFile::AlreadyExists`]，由调用方换一个文件名再试；不会覆盖已有文件。
    fn rename_new(&mut self, from: &Path, to: &Path) -> Result<NewFile, PlatformError>;

    /// 删除文件 `path`。只用于删除本程序自己建的临时文件（例如放弃了的录屏）。
    fn remove_file(&mut self, path: &Path) -> Result<(), PlatformError>;
}
