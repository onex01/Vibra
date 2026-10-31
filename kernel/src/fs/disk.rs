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
use core::sync::atomic::{AtomicU64, Ordering};

/// Счётчики блочного I/O для `top`/`df`: сколько байт реально прошло через
/// VFS. Считаются в `CountingDisk`, то есть на уровне менеджера дисков.
static DISK_READ_BYTES: AtomicU64 = AtomicU64::new(0);
static DISK_WRITE_BYTES: AtomicU64 = AtomicU64::new(0);

pub fn io_stats() -> (u64, u64) {
    (
        DISK_READ_BYTES.load(Ordering::Relaxed),
        DISK_WRITE_BYTES.load(Ordering::Relaxed),
    )
}

/// Прозрачная обёртка, которая считает прочитанные/записанные байты.
pub struct CountingDisk {
    inner: Box<dyn DiskIo>,
}

impl CountingDisk {
    pub fn new(inner: Box<dyn DiskIo>) -> Self {
        Self { inner }
    }
}

impl DiskIo for CountingDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let result = self.inner.read(sector, buf);
        if result.is_ok() {
            DISK_READ_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
        }
        result
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        let result = self.inner.write(sector, buf);
        if result.is_ok() {
            DISK_WRITE_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
        }
        result
    }

    fn sector_size(&self) -> usize {
        self.inner.sector_size()
    }

    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }

    fn read_lba(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let result = self.inner.read_lba(lba, buf);
        if result.is_ok() {
            DISK_READ_BYTES.fetch_add(buf.len() as u64, Ordering::Relaxed);
        }
        result
    }
}

/// Базовая единица адресации block layer: все `DiskIo::read(sector)` считают
/// `sector` в 512-байтных единицах, даже если устройство 4Kn.
pub const SECTOR_SIZE: usize = 512;

/// Трейт для дискового ввода-вывода
pub trait DiskIo: Send + Sync {
    /// Чтение сектора
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError>;
    /// Запись сектора (опционально)
    fn write(&mut self, _sector: u64, _buf: &[u8]) -> Result<(), FsError> {
        // По умолчанию - только чтение
        Err(FsError::ReadOnly)
    }

    /// Устройство только для чтения (CD/DVD, ISO-образ).
    fn is_read_only(&self) -> bool {
        false
    }

    /// Размер логического сектора устройства (512 или 4096).
    ///
    /// FS-слой исторически считает сектора 512-байтными; это значение нужно
    /// для GPT/разделов на 4Kn-дисках.
    fn sector_size(&self) -> usize {
        512
    }

    /// Прочитать один логический сектор устройства по LBA.
    ///
    /// В отличие от `read`, номер `lba` — это номер сектора устройства
    /// (512 или 4096 байт), а не байтовое смещение/512. Нужен для GPT на 4Kn.
    fn read_lba(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FsError> {
        self.read(lba, buf)
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

