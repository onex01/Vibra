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
        }
    }
}

/// Тип файлового объекта
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    File,
    Directory,
}

/// Метаданные файла
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub name: String,
    pub file_type: FileType,
    pub size: usize,
    pub created: u64,
    pub modified: u64,
}

/// Запись в директории
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: usize,
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
    /// Чтение данных из файла
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError>;
    
    /// Запись данных в файл
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError>;
    
    /// Перемещение позиции чтения/записи
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError>;
    
    /// Получение текущей позиции
    fn position(&self) -> u64;
    
    /// Получение размера файла
    fn size(&self) -> usize;
    
    /// Закрытие файла
    fn close(self: Box<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// Трейт для файловой системы
pub trait FileSystem: Send + Sync {
    /// Название файловой системы
    fn name(&self) -> &str;
    
    /// Монтирование ФС
    fn mount(&mut self) -> Result<(), FsError>;
    
    /// Размонтирование ФС
    fn unmount(&mut self) -> Result<(), FsError>;
    
    /// Открытие файла
    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError>;
    
    /// Создание файла
    fn create(&mut self, path: &str) -> Result<Box<dyn File>, FsError>;
    
    /// Удаление файла
    fn remove(&mut self, path: &str) -> Result<(), FsError>;
    
    /// Создание директории
    fn mkdir(&mut self, path: &str) -> Result<(), FsError>;
    
    /// Удаление директории
    fn rmdir(&mut self, path: &str) -> Result<(), FsError>;
    
    /// Чтение содержимого директории
    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError>;
    
    /// Проверка существования пути
    fn exists(&self, path: &str) -> bool;
    
    /// Получение метаданных
    fn stat(&self, path: &str) -> Result<FileMetadata, FsError>;
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
}
