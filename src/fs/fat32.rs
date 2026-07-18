// src/fs/fat32.rs

use super::vfs::*;
use super::disk::DiskIo;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::boxed::Box;
use alloc::vec;

/// FAT32 Boot Sector (первые 512 байт)
#[repr(C, packed)]
struct Fat32BootSector {
    jump: [u8; 3],
    oem_name: [u8; 8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    fat_count: u8,
    root_entry_count: u16,      // Должно быть 0 для FAT32
    total_sectors_16: u16,      // Должно быть 0 для FAT32
    media_type: u8,
    fat_size_16: u16,           // Должно быть 0 для FAT32
    sectors_per_track: u16,
    head_count: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,
    
    // FAT32 специфичные поля
    fat_size_32: u32,
    ext_flags: u16,
    fs_version: u16,
    root_cluster: u32,
    fs_info_sector: u16,
    backup_boot_sector: u16,
    reserved: [u8; 12],
    drive_number: u8,
    reserved1: u8,
    boot_sig: u8,
    volume_id: u32,
    volume_label: [u8; 11],
    fs_type: [u8; 8],
}

/// FAT32 Directory Entry (32 байта)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DirEntry {
    name: [u8; 11],
    attributes: u8,
    reserved: u8,
    creation_time_tenths: u8,
    creation_time: u16,
    creation_date: u16,
    last_access_date: u16,
    first_cluster_high: u16,
    write_time: u16,
    write_date: u16,
    first_cluster_low: u16,
    file_size: u32,
}

impl DirEntry {
    fn is_valid(&self) -> bool {
        self.name[0] != 0x00 && self.name[0] != 0xE5
    }
    
    fn is_directory(&self) -> bool {
        (self.attributes & 0x10) != 0
    }
    
    fn is_file(&self) -> bool {
        !self.is_directory() && (self.attributes & 0x08) == 0 // Не volume label
    }
    
    fn get_cluster(&self) -> u32 {
        ((self.first_cluster_high as u32) << 16) | (self.first_cluster_low as u32)
    }
    
    fn get_name(&self) -> String {
        let mut name = String::new();
        
        // Имя файла (8 символов)
        for &c in &self.name[0..8] {
            if c != b' ' {
                name.push(c as char);
            }
        }
        
        // Расширение (3 символа)
        let ext_start = name.len();
        for &c in &self.name[8..11] {
            if c != b' ' {
                if name.len() == ext_start {
                    name.push('.');
                }
                name.push(c as char);
            }
        }
        
        name
    }
}

/// FAT32 файловая система
pub struct Fat32Fs {
    disk: Box<dyn DiskIo>,
    boot_sector: Fat32BootSector,
    fat_start: u64,
    data_start: u64,
    cluster_size: u32,
    mounted: bool,
}

impl Fat32Fs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        let mut boot_buf = [0u8; 512];
        disk.read(0, &mut boot_buf)?;
        
        let boot_sector: Fat32BootSector = unsafe {
            core::ptr::read(boot_buf.as_ptr() as *const Fat32BootSector)
        };
        
        // Проверяем сигнатуру
        if boot_sector.bytes_per_sector != 512 {
            return Err(FsError::IoError);
        }
        
        let fat_start = (boot_sector.reserved_sectors as u64) * 512;
        let root_dir_sectors = ((boot_sector.root_entry_count * 32) + 511) / 512;
        let fat_size = if boot_sector.fat_size_16 != 0 {
            boot_sector.fat_size_16 as u64
        } else {
            boot_sector.fat_size_32 as u64
        };
        
        let data_start = fat_start + (boot_sector.fat_count as u64 * fat_size * 512) + (root_dir_sectors as u64 * 512);
        let cluster_size = boot_sector.bytes_per_sector as u32 * boot_sector.sectors_per_cluster as u32;
        
        Ok(Self {
            disk,
            boot_sector,
            fat_start,
            data_start,
            cluster_size,
            mounted: false,
        })
    }
    
    /// Чтение кластера
    fn read_cluster(&mut self, cluster: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let sector = self.data_start / 512 + ((cluster - 2) * self.boot_sector.sectors_per_cluster as u32) as u64;
        self.disk.read(sector, buf)
    }
    
    /// Получение следующего кластера из FAT
    fn next_cluster(&mut self, cluster: u32) -> Result<u32, FsError> {
        let fat_offset = cluster as u64 * 4;
        let fat_sector = self.fat_start / 512 + fat_offset / 512;
        let entry_offset = (fat_offset % 512) as usize;
        
        let mut buf = [0u8; 512];
        self.disk.read(fat_sector, &mut buf)?;
        
        let next = u32::from_le_bytes([
            buf[entry_offset],
            buf[entry_offset + 1],
            buf[entry_offset + 2],
            buf[entry_offset + 3],
        ]) & 0x0FFFFFFF;
        
        if next >= 0x0FFFFFF8 {
            Err(FsError::NotFound) // End of chain
        } else {
            Ok(next)
        }
    }
    
    /// Чтение директории
    fn read_directory(&mut self, cluster: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let mut current_cluster = cluster;
        
        loop {
            let mut cluster_buf = vec![0u8; self.cluster_size as usize];
            self.read_cluster(current_cluster, &mut cluster_buf)?;
            
            // Обрабатываем записи директории
            for i in (0..self.cluster_size as usize).step_by(32) {
                let entry: DirEntry = unsafe {
                    core::ptr::read(cluster_buf[i..].as_ptr() as *const DirEntry)
                };
                
                if !entry.is_valid() {
                    if entry.name[0] == 0x00 {
                        return Ok(entries); // End of directory
                    }
                    continue;
                }
                
                if entry.is_file() || entry.is_directory() {
                    entries.push(entry);
                }
            }
            
            // Переходим к следующему кластеру
            match self.next_cluster(current_cluster) {
                Ok(next) => current_cluster = next,
                Err(_) => break,
            }
        }
        
        Ok(entries)
    }
}

pub struct Fat32File {
    data: Vec<u8>,
    position: u64,
    writable: bool,
}

impl File for Fat32File {
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
    
    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
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

impl FileSystem for Fat32Fs {
    fn name(&self) -> &str {
        "fat32"
    }
    
    fn mount(&mut self) -> Result<(), FsError> {
        self.mounted = true;
        Ok(())
    }
    
    fn unmount(&mut self) -> Result<(), FsError> {
        self.mounted = false;
        Ok(())
    }
    
    fn open(&self, _path: &str) -> Result<Box<dyn File>, FsError> {
        Err(FsError::NotFound)
    }
    
    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> {
        Err(FsError::ReadOnly) // Пока только чтение
    }
    
    fn remove(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn mkdir(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn rmdir(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn readdir(&mut self, path: &str) -> Result<Vec<super::vfs::DirEntry>, FsError> {
        if path != "/" {
            return Err(FsError::NotFound);
        }
        
        let mut entries = Vec::new();
        
        // Читаем корневую директорию
        let root_cluster = self.boot_sector.root_cluster;
        
        match self.read_directory(root_cluster) {
            Ok(dir_entries) => {
                for entry in dir_entries {
                    if entry.is_file() {
                        entries.push(super::vfs::DirEntry {
                            name: entry.get_name(),
                            file_type: FileType::File,
                            size: entry.file_size as usize,
                        });
                    } else if entry.is_directory() {
                        entries.push(super::vfs::DirEntry {
                            name: entry.get_name(),
                            file_type: FileType::Directory,
                            size: 0,
                        });
                    }
                }
                Ok(entries)
            }
            Err(_) => Err(FsError::IoError)
        }
    }
    
    fn exists(&self, _path: &str) -> bool {
        false
    }
    
    fn stat(&self, _path: &str) -> Result<FileMetadata, FsError> {
        Err(FsError::NotFound)
    }
}