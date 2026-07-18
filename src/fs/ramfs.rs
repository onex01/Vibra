// src/fs/ramfs.rs

use super::vfs::*;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::boxed::Box;

/// Внутреннее представление записи
#[derive(Clone)]
struct RamFsEntry {
    name: String,
    file_type: FileType,
    data: Vec<u8>,
    created: u64,
    modified: u64,
}

impl RamFsEntry {
    fn new_file(name: &str) -> Self {
        RamFsEntry {
            name: String::from(name),
            file_type: FileType::File,
            data: Vec::new(),
            created: 0,
            modified: 0,
        }
    }
    
    fn new_dir(name: &str) -> Self {
        RamFsEntry {
            name: String::from(name),
            file_type: FileType::Directory,
            data: Vec::new(),
            created: 0,
            modified: 0,
        }
    }
}

/// Файловая система в памяти
pub struct RamFs {
    entries: Vec<RamFsEntry>,
    mounted: bool,
}

impl RamFs {
    pub fn new() -> Self {
        RamFs {
            entries: Vec::new(),
            mounted: false,
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }
    
    fn mount(&mut self) -> Result<(), FsError> {
        self.mounted = true;
        // Создаём корневую директорию
        if !self.entries.iter().any(|e| e.name == "/") {
            self.entries.push(RamFsEntry::new_dir("/"));
        }
        Ok(())
    }
    
    fn unmount(&mut self) -> Result<(), FsError> {
        self.mounted = false;
        Ok(())
    }
    
    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let path = normalize_path(path);
        
        for entry in &self.entries {
            if entry.name == path && entry.file_type == FileType::File {
                return Ok(Box::new(RamFsFile {
                    data: entry.data.clone(),
                    position: 0,
                    writable: true,
                }));
            }
        }
        
        Err(FsError::NotFound)
    }
    
    fn create(&mut self, path: &str) -> Result<Box<dyn File>, FsError> {
        let path = normalize_path(path);
        
        // Проверяем, не существует ли уже
        if self.entries.iter().any(|e| e.name == path) {
            return Err(FsError::AlreadyExists);
        }
        
        self.entries.push(RamFsEntry::new_file(&path));
        
        Ok(Box::new(RamFsFile {
            data: Vec::new(),
            position: 0,
            writable: true,
        }))
    }
    
    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        
        if let Some(pos) = self.entries.iter().position(|e| e.name == path) {
            self.entries.remove(pos);
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }
    
    fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        
        if self.entries.iter().any(|e| e.name == path) {
            return Err(FsError::AlreadyExists);
        }
        
        self.entries.push(RamFsEntry::new_dir(&path));
        Ok(())
    }
    
    fn rmdir(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        
        // Проверяем, пуста ли директория
        let has_children = self.entries.iter().any(|e| {
            e.name.starts_with(&path) && e.name != path
        });
        
        if has_children {
            return Err(FsError::NotEmpty);
        }
        
        if let Some(pos) = self.entries.iter().position(|e| e.name == path) {
            self.entries.remove(pos);
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }
    
    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = normalize_path(path);
        let mut entries = Vec::new();
        
        for entry in &self.entries {
            // Ищем прямых потомков
            if entry.name.starts_with(&path) && entry.name != path {
                let relative = &entry.name[path.len()..];
                // Только прямые потомки (без '/')
                if !relative.contains('/') || (relative == "/" && path == "/") {
                    entries.push(DirEntry {
                        name: relative.trim_matches('/').to_string(),
                        file_type: entry.file_type,
                        size: entry.data.len(),
                    });
                }
            }
        }
        
        Ok(entries)
    }
    
    fn exists(&self, path: &str) -> bool {
        let path = normalize_path(path);
        self.entries.iter().any(|e| e.name == path)
    }
    
    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let path = normalize_path(path);
        
        for entry in &self.entries {
            if entry.name == path {
                return Ok(FileMetadata {
                    name: entry.name.clone(),
                    file_type: entry.file_type,
                    size: entry.data.len(),
                    created: entry.created,
                    modified: entry.modified,
                });
            }
        }
        
        Err(FsError::NotFound)
    }
}

/// Файл в RamFS
pub struct RamFsFile {
    data: Vec<u8>,
    position: u64,
    writable: bool,
}

impl File for RamFsFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() {
            return Ok(0);
        }
        
        let available = self.data.len() - pos;
        let to_read = buf.len().min(available);
        
        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;
        
        Ok(to_read)
    }
    
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        if !self.writable {
            return Err(FsError::ReadOnly);
        }
        
        let pos = self.position as usize;
        
        // Расширяем буфер если нужно
        if pos + buf.len() > self.data.len() {
            self.data.resize(pos + buf.len(), 0);
        }
        
        self.data[pos..pos + buf.len()].copy_from_slice(buf);
        self.position += buf.len() as u64;
        
        Ok(buf.len())
    }
    
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(offset) => self.position as i64 + offset,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
        };
        
        if new_pos < 0 {
            return Err(FsError::InvalidPath);
        }
        
        self.position = new_pos as u64;
        Ok(self.position)
    }
    
    fn position(&self) -> u64 {
        self.position
    }
    
    fn size(&self) -> usize {
        self.data.len()
    }
}

/// Нормализация пути
fn normalize_path(path: &str) -> String {
    let mut normalized = String::from("/");
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    
    for part in parts {
        if part == "." {
            continue;
        } else if part == ".." {
            // Упрощённая обработка ..
            continue;
        } else {
            if !normalized.ends_with('/') {
                normalized.push('/');
            }
            normalized.push_str(part);
        }
    }
    
    normalized
}