// src/fs/disk.rs - Disk I/O interface for FAT32/EXT filesystems
//
// Реализует трейт DiskIo для чтения/записи секторов с диска.
// Пока поддерживает только чтение через RAMFS-совместимый интерфейс.
// Для FAT32/EXT2/3/4 потребуется реальный драйвер диска (ATA/IDE/SATA).

use super::vfs::FsError;
use crate::println;
use alloc::vec;
use alloc::vec::Vec;
use alloc::boxed::Box;

/// Трейт для дискового ввода-вывода
pub trait DiskIo: Send + Sync {
    /// Чтение сектора
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError>;
    /// Запись сектора (опционально)
    fn write(&mut self, _sector: u64, _buf: &[u8]) -> Result<(), FsError> {
        // По умолчанию - только чтение
        Err(FsError::ReadOnly)
    }
}

/// RAM-based disk для тестирования (всё в памяти)
pub struct RamDisk {
    data: Vec<u8>,
    sector_size: usize,
}

impl RamDisk {
    pub fn new(size_mb: usize) -> Self {
        let size = size_mb * 1024 * 1024;
        println!("[DISK] RAM disk {} MB allocated", size_mb);
        Self {
            data: vec![0u8; size],
            sector_size: 512,
        }
    }
    
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let sector_size = 512;
        println!("[DISK] RAM disk created from bytes ({} MB)", data.len() / (1024 * 1024));
        Self {
            data,
            sector_size,
        }
    }
}

impl DiskIo for RamDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let offset = (sector * self.sector_size as u64) as usize;
        let end = offset + buf.len();
        
        if end > self.data.len() {
            println!("[DISK] Read error: sector {} out of bounds", sector);
            return Err(FsError::IoError);
        }
        
        buf.copy_from_slice(&self.data[offset..end]);
        Ok(())
    }
    
    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        let offset = (sector * self.sector_size as u64) as usize;
        let end = offset + buf.len();
        
        if end > self.data.len() {
            println!("[DISK] Write error: sector {} out of bounds", sector);
            return Err(FsError::IoError);
        }
        
        self.data[offset..end].copy_from_slice(buf);
        Ok(())
    }
}

/// FatDisk - обёртка для FAT32 файловой системы
/// Позволяет монтировать FAT32 поверх реального диска
pub struct FatDisk {
    disk: Box<dyn DiskIo>,
}

impl FatDisk {
    pub fn new(disk: Box<dyn DiskIo>) -> Self {
        Self { disk }
    }
    
    pub fn get_disk(&mut self) -> &mut dyn DiskIo {
        self.disk.as_mut()
    }
}

