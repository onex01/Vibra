// FAT32 File System Driver — чтение/запись FAT32 разделов

use super::vfs::*;
use super::disk::DiskIo;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::boxed::Box;
use alloc::vec;

const SECTOR_SIZE: usize = 512;

/// FAT32 Boot Sector (BPB).
///
/// Первые 11 байт — `jmp_boot[3]` + `oem_name[8]`; без них все поля BPB
/// читались со сдвигом на 11 байт, `bytes_per_sector` оказывался мусором и
/// монтирование любого FAT32 падало с InvalidPath.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Fat32BootSector {
    _jump: [u8; 3],
    _oem: [u8; 8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    fat_count: u8,
    root_entry_count: u16,
    total_sectors_16: u16,
    media_type: u8,
    fat_size_16: u16,
    sectors_per_track: u16,
    head_count: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,
    fat_size_32: u32,
    ext_flags: u16,
    fs_version: u16,
    root_cluster: u32,
    fs_info_sector: u16,
    backup_boot_sector: u16,
    drive_number: u8,
    volume_label: [u8; 11],
    fs_type: [u8; 8],
}

/// FAT32 Directory Entry (32 bytes)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FatDirEntry {
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

impl FatDirEntry {
    fn is_directory(&self) -> bool { (self.attributes & 0x10) != 0 }
    fn is_file(&self) -> bool { !self.is_directory() && (self.attributes & 0x08) == 0 }
    fn is_long_name(&self) -> bool { (self.attributes & 0x0F) == 0x0F }
    fn get_cluster(&self) -> u32 {
        ((self.first_cluster_high as u32) << 16) | (self.first_cluster_low as u32)
    }
    fn get_name(&self) -> String {
        let mut name = String::new();
        for &c in &self.name[0..8] {
            if c != b' ' { name.push(c as char); }
        }
        let ext_start = name.len();
        for &c in &self.name[8..11] {
            if c != b' ' {
                if name.len() == ext_start { name.push('.'); }
                name.push(c as char);
            }
        }
        name
    }
}

pub struct Fat32Fs {
    disk: spin::Mutex<Box<dyn DiskIo>>,
    boot: Fat32BootSector,
    fat_start_sector: u64,
    data_start_sector: u64,
    cluster_size: u32,
    mounted: bool,
}

impl Fat32Fs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        let mut buf = [0u8; 512];
        disk.read(0, &mut buf).map_err(|_| FsError::IoError)?;

        let boot: Fat32BootSector = unsafe { core::ptr::read(buf.as_ptr() as *const _) };

        // FAT32: fat_size_16 == 0 и root_entry_count == 0 (корневой каталог —
        // обычная цепочка кластеров, а не фиксированная область). Если это не
        // так, перед нами FAT12/16 и раскладка data-области другая.
        if boot.bytes_per_sector != 512 || boot.fat_size_16 != 0 || boot.root_entry_count != 0 {
            return Err(FsError::InvalidPath);
        }
        if boot.sectors_per_cluster == 0 || !boot.sectors_per_cluster.is_power_of_two() {
            return Err(FsError::InvalidPath);
        }

        let fat_start = boot.reserved_sectors as u64;
        let fat_size = boot.fat_size_32 as u64;
        let data_start = fat_start + (boot.fat_count as u64 * fat_size)
            + ((boot.root_entry_count as u64 * 32 + 511) / 512);
        let cluster_size = boot.bytes_per_sector as u32 * boot.sectors_per_cluster as u32;

        Ok(Self {
            disk: spin::Mutex::new(disk),
            boot,
            fat_start_sector: fat_start,
            data_start_sector: data_start,
            cluster_size,
            mounted: false,
        })
    }

    fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<(), FsError> {
        // Кластеры 0 и 1 — служебные (0 = свободно, 1 = зарезервировано),
        // данных в них нет. Без этой проверки `cluster - 2` уходил в underflow.
        if cluster < 2 { return Err(FsError::InvalidPath); }
        let sector = self.data_start_sector
            + ((cluster - 2) as u64) * self.boot.sectors_per_cluster as u64;
        self.disk.lock().read(sector, buf).map_err(|_| FsError::IoError)
    }

    fn next_cluster(&self, cluster: u32) -> Result<u32, FsError> {
        let fat_offset = cluster as u64 * 4;
        let fat_sector = self.fat_start_sector + fat_offset / 512;
        let entry_offset = (fat_offset % 512) as usize;

        let mut buf = [0u8; 512];
        self.disk.lock().read(fat_sector, &mut buf).map_err(|_| FsError::IoError)?;

        let next = u32::from_le_bytes([
            buf[entry_offset], buf[entry_offset+1],
            buf[entry_offset+2], buf[entry_offset+3],
        ]) & 0x0FFFFFFF;

        if next >= 0x0FFFFFF8 { Err(FsError::NotFound) } else { Ok(next) }
    }

    fn read_chain(&self, start_cluster: u32) -> Result<Vec<u8>, FsError> {
        let mut data = Vec::new();
        // Пустой файл (cluster == 0) — данных нет.
        if start_cluster < 2 { return Ok(data); }

        let mut cluster = start_cluster;
        let mut cluster_buf = vec![0u8; self.cluster_size as usize];
        // Защита от битой/зацикленной FAT: не больше числа кластеров тома.
        let max_clusters = (self.boot.fat_size_32 as u64 * 512 / 4) as usize;
        let mut guard = 0usize;

        loop {
            self.read_cluster(cluster, &mut cluster_buf)?;
            data.extend_from_slice(&cluster_buf);
            guard += 1;
            if guard > max_clusters { break; }
            match self.next_cluster(cluster) {
                Ok(next) => cluster = next,
                Err(_) => break,
            }
        }
        Ok(data)
    }

    /// Читает каталог и возвращает записи вместе с именами.
    ///
    /// Поддержаны LFN (Long File Name): цепочка записей с атрибутом 0x0F идёт
    /// ПЕРЕД короткой записью и хранит по 13 UTF-16 символов; части склеиваются
    /// в обратном порядке. Без этого файлы вида "My Documents" выглядели как
    /// "MYDOCU~1".
    fn read_dir_cluster(&self, cluster: u32) -> Result<Vec<(FatDirEntry, String)>, FsError> {
        let data = self.read_chain(cluster)?;
        let mut entries: Vec<(FatDirEntry, String)> = Vec::new();
        let mut lfn_parts: Vec<[u16; 13]> = Vec::new();
        let mut i = 0usize;

        while i + 32 <= data.len() {
            let entry: FatDirEntry = unsafe { core::ptr::read(data[i..].as_ptr() as *const _) };

            if entry.name[0] == 0x00 {
                break;
            }
            if entry.name[0] == 0xE5 {
                // Удалённая запись обрывает LFN-цепочку.
                lfn_parts.clear();
                i += 32;
                continue;
            }

            if entry.is_long_name() {
                let mut chars = [0u16; 13];
                for k in 0..5 {
                    chars[k] = u16::from_le_bytes([data[i + 1 + k * 2], data[i + 2 + k * 2]]);
                }
                for k in 0..6 {
                    chars[5 + k] = u16::from_le_bytes([data[i + 14 + k * 2], data[i + 15 + k * 2]]);
                }
                chars[11] = u16::from_le_bytes([data[i + 28], data[i + 29]]);
                chars[12] = u16::from_le_bytes([data[i + 30], data[i + 31]]);
                lfn_parts.push(chars);
                i += 32;
                continue;
            }

            let name = if lfn_parts.is_empty() {
                entry.get_name()
            } else {
                let mut s = String::new();
                for part in lfn_parts.iter().rev() {
                    for &c in part.iter() {
                        // 0x0000 — терминатор, 0xFFFF — padding.
                        if c == 0 || c == 0xFFFF {
                            continue;
                        }
                        s.push(char::from_u32(c as u32).unwrap_or('?'));
                    }
                }
                lfn_parts.clear();
                if s.is_empty() { entry.get_name() } else { s }
            };

            // 0x08 = volume label, в листинг не попадает.
            if entry.attributes & 0x08 == 0 {
                entries.push((entry, name));
            }
            i += 32;
        }

        Ok(entries)
    }

    fn find_in_dir(&self, dir_cluster: u32, name: &str) -> Result<FatDirEntry, FsError> {
        let entries = self.read_dir_cluster(dir_cluster)?;
        for (entry, entry_name) in &entries {
            if entry_name.eq_ignore_ascii_case(name) {
                return Ok(*entry);
            }
        }
        Err(FsError::NotFound)
    }

    fn find_path(&self, path: &str) -> Result<FatDirEntry, FsError> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            // Корень: псевдо-запись, указывающая на root_cluster.
            // Старшие 16 бит кластера раньше терялись (root_cluster as u16).
            return Ok(FatDirEntry {
                name: [b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                attributes: 0x10,
                first_cluster_high: (self.boot.root_cluster >> 16) as u16,
                first_cluster_low: (self.boot.root_cluster & 0xFFFF) as u16,
                file_size: 0,
                ..unsafe { core::mem::zeroed() }
            });
        }

        let mut current_cluster = self.boot.root_cluster;
        for (i, part) in parts.iter().enumerate() {
            let entry = self.find_in_dir(current_cluster, part)?;
            if i == parts.len() - 1 { return Ok(entry); }
            if entry.is_directory() {
                current_cluster = entry.get_cluster();
            } else {
                return Err(FsError::NotADirectory);
            }
        }
        Err(FsError::NotFound)
    }
}

/// FAT32 файл (только чтение)
pub struct Fat32File {
    data: Vec<u8>,
    position: u64,
}

impl File for Fat32File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() { return Ok(0); }
        let to_read = buf.len().min(self.data.len() - pos);
        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }
    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
    }
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::Current(o) => self.position as i64 + o,
            SeekFrom::End(o) => self.data.len() as i64 + o,
        };
        if new < 0 { return Err(FsError::InvalidPath); }
        self.position = new as u64;
        Ok(self.position)
    }
    fn position(&self) -> u64 { self.position }
    fn size(&self) -> usize { self.data.len() }
}

impl FileSystem for Fat32Fs {
    fn name(&self) -> &str { "fat32" }
    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let entry = self.find_path(path)?;
        if entry.is_file() {
            let data = self.read_chain(entry.get_cluster())?;
            let size = entry.file_size as usize;
            let data = if size < data.len() { data[..size].to_vec() } else { data };
            Ok(Box::new(Fat32File { data, position: 0 }))
        } else if entry.is_directory() {
            Err(FsError::IsADirectory)
        } else {
            Err(FsError::NotFound)
        }
    }

    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> {
        // Запись на физический носитель по умолчанию выключена (см. roadmap:
        // «Операции записи ... выключены по умолчанию»). FAT32 смонтирован
        // read-only, поэтому создание файлов недоступно.
        Err(FsError::ReadOnly)
    }
    fn remove(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let entry = self.find_path(path)?;
        let cluster = if path == "/" || path.is_empty() {
            self.boot.root_cluster
        } else {
            if !entry.is_directory() { return Err(FsError::NotADirectory); }
            entry.get_cluster()
        };

        let fat_entries = self.read_dir_cluster(cluster)?;
        let mut result = Vec::new();

        for (e, name) in &fat_entries {
            if name == "." || name == ".." || name.is_empty() { continue; }
            let file_type = if e.is_directory() { FileType::Directory } else { FileType::File };
            let perms = if e.is_directory() { 0o755 } else { 0o644 };

            result.push(DirEntry {
                name: name.clone(),
                file_type,
                size: e.file_size as usize,
                permissions: Permissions::new(perms),
                uid: 0,
                gid: 0,
            });
        }
        Ok(result)
    }

    fn exists(&self, path: &str) -> bool {
        self.find_path(path).is_ok()
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let entry = self.find_path(path)?;
        // Имя берём из пути: у записи может быть LFN, а get_name() вернёт 8.3.
        let name = String::from(path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/"));
        let file_type = if entry.is_directory() { FileType::Directory } else { FileType::File };
        let perms = if entry.is_directory() { 0o755 } else { 0o644 };

        Ok(FileMetadata {
            name, file_type, size: entry.file_size as usize,
            permissions: Permissions::new(perms), uid: 0, gid: 0, created: 0, modified: 0,
        })
    }
}
