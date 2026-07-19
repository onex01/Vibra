// VFS — Virtual File System для Vibra OS
//
// Единый интерфейс для всех файловых систем.
// Поддерживает: права доступа, ownership, symbolic links (планируется).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::fmt;
use super::mount::MountTable;

/// Ошибки файловой системы
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    IoError,
    InvalidPath,
    AlreadyExists,
    NotEmpty,
    NotADirectory,
    IsADirectory,
    DiskFull,
    ReadOnly,
    NotSupported,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::NotFound => write!(f, "File or directory not found"),
            FsError::PermissionDenied => write!(f, "Permission denied"),
            FsError::IoError => write!(f, "I/O error"),
            FsError::InvalidPath => write!(f, "Invalid path"),
            FsError::AlreadyExists => write!(f, "File already exists"),
            FsError::NotEmpty => write!(f, "Directory not empty"),
            FsError::NotADirectory => write!(f, "Not a directory"),
            FsError::IsADirectory => write!(f, "Is a directory"),
            FsError::DiskFull => write!(f, "Disk full"),
            FsError::ReadOnly => write!(f, "Read-only file system"),
            FsError::NotSupported => write!(f, "Operation not supported"),
        }
    }
}

/// Тип файлового объекта
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Device,
}

/// Права доступа (rwxrwxrwx)
#[derive(Debug, Clone, Copy)]
pub struct Permissions {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_exec: bool,
    pub group_read: bool,
    pub group_write: bool,
    pub group_exec: bool,
    pub other_read: bool,
    pub other_write: bool,
    pub other_exec: bool,
}

impl Permissions {
    pub fn new(rwx: u16) -> Self {
        Self {
            owner_read: (rwx & 0o400) != 0,
            owner_write: (rwx & 0o200) != 0,
            owner_exec: (rwx & 0o100) != 0,
            group_read: (rwx & 0o040) != 0,
            group_write: (rwx & 0o020) != 0,
            group_exec: (rwx & 0o010) != 0,
            other_read: (rwx & 0o004) != 0,
            other_write: (rwx & 0o002) != 0,
            other_exec: (rwx & 0o001) != 0,
        }
    }

    pub fn to_octal(&self) -> u16 {
        let mut mode = 0u16;
        if self.owner_read { mode |= 0o400; }
        if self.owner_write { mode |= 0o200; }
        if self.owner_exec { mode |= 0o100; }
        if self.group_read { mode |= 0o040; }
        if self.group_write { mode |= 0o020; }
        if self.group_exec { mode |= 0o010; }
        if self.other_read { mode |= 0o004; }
        if self.other_write { mode |= 0o002; }
        if self.other_exec { mode |= 0o001; }
        mode
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        s.push(if self.owner_read { 'r' } else { '-' });
        s.push(if self.owner_write { 'w' } else { '-' });
        s.push(if self.owner_exec { 'x' } else { '-' });
        s.push(if self.group_read { 'r' } else { '-' });
        s.push(if self.group_write { 'w' } else { '-' });
        s.push(if self.group_exec { 'x' } else { '-' });
        s.push(if self.other_read { 'r' } else { '-' });
        s.push(if self.other_write { 'w' } else { '-' });
        s.push(if self.other_exec { 'x' } else { '-' });
        s
    }
}

/// Метаданные файла
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub name: String,
    pub file_type: FileType,
    pub size: usize,
    pub permissions: Permissions,
    pub uid: u32,
    pub gid: u32,
    pub created: u64,
    pub modified: u64,
}

/// Запись в директории
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: usize,
    pub permissions: Permissions,
    pub uid: u32,
    pub gid: u32,
}

/// Позиция для seek
#[derive(Debug, Clone, Copy)]
pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}

/// Трейт для файлового объекта
pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError>;
    fn position(&self) -> u64;
    fn size(&self) -> usize;
    fn close(self: Box<Self>) -> Result<(), FsError> { Ok(()) }
}

/// Трейт для файловой системы
pub trait FileSystem: Send + Sync {
    fn name(&self) -> &str;
    fn mount(&mut self) -> Result<(), FsError>;
    fn unmount(&mut self) -> Result<(), FsError>;
    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError>;
    fn create(&mut self, path: &str) -> Result<Box<dyn File>, FsError>;
    fn remove(&mut self, path: &str) -> Result<(), FsError>;
    fn mkdir(&mut self, path: &str) -> Result<(), FsError>;
    fn rmdir(&mut self, path: &str) -> Result<(), FsError>;
    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError>;
    fn exists(&self, path: &str) -> bool;
    fn stat(&self, path: &str) -> Result<FileMetadata, FsError>;
    fn chmod(&mut self, _path: &str, _mode: u16) -> Result<(), FsError> { Err(FsError::NotSupported) }
    fn chown(&mut self, _path: &str, _uid: u32, _gid: u32) -> Result<(), FsError> { Err(FsError::NotSupported) }
}

/// Менеджер виртуальной файловой системы
pub struct VfsManager {
    pub mount_table: spin::Mutex<MountTable>,
}

impl VfsManager {
    pub fn new() -> Self {
        VfsManager {
            mount_table: spin::Mutex::new(MountTable::new()),
        }
    }
    
    pub fn mount(&self, path: &str, fs: Box<dyn FileSystem>, readonly: bool) -> Result<(), FsError> {
        self.mount_table.lock().mount(path, fs, readonly)
    }

    /// Найти ФС для пути и вернуть (индекс, относительный путь)
    pub fn resolve(&self, path: &str) -> Option<(usize, String)> {
        self.mount_table.lock().find_fs(path)
    }
}
