// src/fs/disk_manager.rs - Disk management for mounting filesystems
//
// Предоставляет менеджер дисков для монтирования различных ФС.
// Позволяет добавлять диски и получать к ним доступ по индексу.

use super::disk::DiskIo;
use alloc::vec::Vec;
use alloc::boxed::Box;

/// Менеджер дисков для монтирования
pub struct DiskManager {
    disks: Vec<Box<dyn DiskIo>>,
}

impl DiskManager {
    pub fn new() -> Self {
        Self { disks: Vec::new() }
    }
    
    /// Добавить диск
    pub fn add_disk(&mut self, disk: Box<dyn DiskIo>) {
        self.disks.push(disk);
    }
    
    /// Получить диск по индексу
    pub fn get_disk(&mut self, index: usize) -> Option<&mut (dyn DiskIo + 'static)> {
        self.disks.get_mut(index).map(|d| d.as_mut())
    }
    
    /// Количество дисков
    pub fn count(&self) -> usize {
        self.disks.len()
    }
}

impl Default for DiskManager {
    fn default() -> Self {
        Self::new()
    }
}
