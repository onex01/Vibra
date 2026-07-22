// src/fs/mount.rs - Mount point management
//
// Управляет точками монтирования файловых систем.
// Позволяет монтировать несколько ФС в единую иерархию.

use super::vfs::{FileSystem, FsError};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::format;
use core::fmt;

/// Точка монтирования
pub struct MountPoint {
    pub path: String,
    pub fs: Box<dyn FileSystem>,
    pub readonly: bool,
}

impl MountPoint {
    pub fn new(path: &str, fs: Box<dyn FileSystem>, readonly: bool) -> Self {
        Self {
            path: String::from(path),
            fs,
            readonly,
        }
    }
}

impl fmt::Display for MountPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {} (readonly: {})", self.path, self.fs.name(), self.readonly)
    }
}

/// Таблица монтирования
pub struct MountTable {
    mounts: Vec<MountPoint>,
}

impl MountTable {
    pub const fn new() -> Self {
        Self { mounts: Vec::new() }
    }
    
    /// Монтирование файловой системы
    pub fn mount(&mut self, path: &str, fs: Box<dyn FileSystem>, readonly: bool) -> Result<(), FsError> {
        // Нормализуем путь
        let path = normalize_path(path);
        
        // Проверяем, что путь ещё не занят
        for mp in &self.mounts {
            if mp.path == path {
                return Err(FsError::AlreadyExists);
            }
        }
        
        self.mounts.push(MountPoint::new(&path, fs, readonly));
        Ok(())
    }
    
    /// Универсальное монтирование (по умолчанию read-write)
    pub fn mount_rw(&mut self, path: &str, fs: Box<dyn FileSystem>) -> Result<(), FsError> {
        self.mount(path, fs, false)
    }
    
    /// Монтирование только для чтения
    pub fn mount_ro(&mut self, path: &str, fs: Box<dyn FileSystem>) -> Result<(), FsError> {
        self.mount(path, fs, true)
    }
    
    /// Размонтирование
    pub fn unmount(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        if let Some(pos) = self.mounts.iter().position(|mp| mp.path == path) {
            self.mounts.remove(pos);
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }
    
    /// Найти ФС для пути
    /// Возвращает (индекс точки монтирования, относительный путь внутри ФС)
    pub fn find_fs(&self, path: &str) -> Option<(usize, String)> {
        let path = normalize_path(path);
        let mut best_match: Option<(usize, String)> = None;
        let mut best_len = 0;

        for (idx, mp) in self.mounts.iter().enumerate() {
            if path.starts_with(&mp.path) {
                // Вычисляем относительный путь
                let relative = if path.len() > mp.path.len() {
                    &path[mp.path.len()..]
                } else {
                    ""
                };

                if mp.path.len() > best_len {
                    best_match = Some((idx, relative.to_string()));
                    best_len = mp.path.len();
                }
            }
        }

        best_match
    }

    /// Получить точку монтирования по индексу
    pub fn get(&self, idx: usize) -> &MountPoint {
        &self.mounts[idx]
    }

    /// Получить точку монтирования по индексу (изменяемо)
    pub fn get_mut(&mut self, idx: usize) -> &mut MountPoint {
        &mut self.mounts[idx]
    }

    /// Список всех точек монтирования (строки для вывода)
    pub fn list(&self) -> Vec<String> {
        self.mounts.iter().map(|m| format!("{}", m)).collect()
    }

    /// Проверить, смонтирован ли путь
    pub fn is_mounted(&self, path: &str) -> bool {
        let path = normalize_path(path);
        self.mounts.iter().any(|m| m.path == path)
    }
    
    /// Количество смонтированных ФС
    pub fn count(&self) -> usize {
        self.mounts.len()
    }
}

impl Default for MountTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Нормализация пути
fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    
    let mut normalized = String::new();
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    
    for part in parts {
        if part == "." {
            continue;
        } else if part == ".." {
            // Удаляем последний компонент
            while normalized.ends_with('/') {
                normalized.pop();
            }
            while !normalized.is_empty() && !normalized.ends_with('/') {
                normalized.pop();
            }
        } else {
            normalized.push('/');
            normalized.push_str(part);
        }
    }
    
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}
