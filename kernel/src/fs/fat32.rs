// FAT32 File System Driver — чтение и запись FAT32.
//
// Раскладка тома:
//   [reserved][FAT#1..FAT#n][data clusters...]
// Корневой каталог — обычная цепочка кластеров (`root_cluster`).
//
// Все низкоуровневые операции (сектор, запись элемента FAT во ВСЕ копии,
// аллокация/освобождение кластеров, запись по смещению с продлением цепочки)
// реализованы свободными функциями над `Fat32Geo`: один и тот же код
// используют и VFS-обёртка `Fat32Fs`, и `Fat32File`, и приёмник boot-лога.
//
// Запись по умолчанию выключена (`new`), включается `new_rw`: монтирование
// загрузочной флешки не должно её менять без явного согласия.

use super::disk::DiskIo;
use super::vfs::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

const SECTOR_SIZE: usize = 512;
const FAT_EOC: u32 = 0x0FFF_FFFF;
const FAT_FREE: u32 = 0x0000_0000;
const FAT_MASK: u32 = 0x0FFF_FFFF;

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
    /// FAT32: 12 зарезервированных байт (0x34..0x3F). Без них `drive_number`
    /// и `volume_label` читались со сдвигом на 12 байт, и метка тома была
    /// нулевой — поиск раздела VIBRALOG не работал.
    _reserved: [u8; 12],
    drive_number: u8,
    _reserved1: u8,
    /// 0x29 = extended boot signature (volume id/label присутствуют).
    _boot_signature: u8,
    _volume_id: u32,
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
    const ATTR_DIRECTORY: u8 = 0x10;
    const ATTR_VOLUME_ID: u8 = 0x08;
    const ATTR_LONG_NAME: u8 = 0x0F;

    fn is_directory(&self) -> bool {
        (self.attributes & Self::ATTR_DIRECTORY) != 0
    }
    fn is_file(&self) -> bool {
        !self.is_directory() && (self.attributes & Self::ATTR_VOLUME_ID) == 0
    }
    fn is_long_name(&self) -> bool {
        (self.attributes & Self::ATTR_LONG_NAME) == Self::ATTR_LONG_NAME
    }
    fn is_free(&self) -> bool {
        self.name[0] == 0x00 || self.name[0] == 0xE5
    }
    fn is_end(&self) -> bool {
        self.name[0] == 0x00
    }
    fn get_cluster(&self) -> u32 {
        ((self.first_cluster_high as u32) << 16) | (self.first_cluster_low as u32)
    }
    fn set_cluster(&mut self, cluster: u32) {
        self.first_cluster_high = (cluster >> 16) as u16;
        self.first_cluster_low = (cluster & 0xFFFF) as u16;
    }
    fn get_name(&self) -> String {
        let mut name = String::new();
        for &c in &self.name[0..8] {
            if c != b' ' {
                name.push(c as char);
            }
        }
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

/// Геометрия тома: всё, что нужно для адресации секторов и кластеров.
#[derive(Clone, Copy)]
pub struct Fat32Geo {
    pub fat_start_sector: u64,
    pub fat_count: u8,
    pub fat_size: u32,
    pub data_start_sector: u64,
    pub sectors_per_cluster: u8,
    pub cluster_size: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    /// Полное число кластеров данных (кластеры 2..2+total_clusters).
    pub total_clusters: u32,
    pub volume_label: [u8; 11],
}

impl Fat32Geo {
    pub fn cluster_to_sector(&self, cluster: u32) -> Option<u64> {
        if cluster < 2 {
            return None;
        }
        Some(self.data_start_sector + ((cluster - 2) as u64) * self.sectors_per_cluster as u64)
    }
}

fn read_sector(disk: &Mutex<Box<dyn DiskIo>>, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
    disk.lock().read(sector, buf).map_err(|_| FsError::IoError)
}

fn write_sector(disk: &Mutex<Box<dyn DiskIo>>, sector: u64, buf: &[u8]) -> Result<(), FsError> {
    disk.lock().write(sector, buf).map_err(|_| FsError::IoError)
}

// ==========================================
// FAT: чтение/запись элементов
// ==========================================

fn fat_get(disk: &Mutex<Box<dyn DiskIo>>, geo: &Fat32Geo, cluster: u32) -> Result<u32, FsError> {
    let offset = cluster as u64 * 4;
    let sector = geo.fat_start_sector + offset / SECTOR_SIZE as u64;
    let entry = (offset % SECTOR_SIZE as u64) as usize;

    let mut buf = [0u8; SECTOR_SIZE];
    read_sector(disk, sector, &mut buf)?;
    Ok(u32::from_le_bytes([
        buf[entry],
        buf[entry + 1],
        buf[entry + 2],
        buf[entry + 3],
    ]) & FAT_MASK)
}

/// Записывает элемент FAT во **все** копии таблицы.
///
/// Если писать только в первую копию, Windows/chkdsk видят «грязный» том,
/// а вторая копия остаётся с мусором.
fn fat_set(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    cluster: u32,
    value: u32,
) -> Result<(), FsError> {
    let offset = cluster as u64 * 4;
    let sector_in_fat = offset / SECTOR_SIZE as u64;
    let entry = (offset % SECTOR_SIZE as u64) as usize;

    let mut buf = [0u8; SECTOR_SIZE];
    read_sector(disk, geo.fat_start_sector + sector_in_fat, &mut buf)?;
    let old = u32::from_le_bytes([buf[entry], buf[entry + 1], buf[entry + 2], buf[entry + 3]]);
    // Старшие 4 бита зарезервированы и должны сохраняться.
    let updated = (old & !FAT_MASK) | (value & FAT_MASK);
    buf[entry..entry + 4].copy_from_slice(&updated.to_le_bytes());

    for copy in 0..geo.fat_count.max(1) as u64 {
        write_sector(
            disk,
            geo.fat_start_sector + copy * geo.fat_size as u64 + sector_in_fat,
            &buf,
        )?;
    }
    Ok(())
}

fn is_end_of_chain(value: u32) -> bool {
    value >= 0x0FFF_FFF8
}

/// Цепочка кластеров файла (без служебных).
pub fn chain(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    start: u32,
) -> Result<Vec<u32>, FsError> {
    let mut result = Vec::new();
    if start < 2 {
        return Ok(result);
    }

    let mut cluster = start;
    let limit = geo.total_clusters as usize + 2;
    while result.len() <= limit {
        result.push(cluster);
        let next = fat_get(disk, geo, cluster)?;
        if next < 2 || is_end_of_chain(next) {
            break;
        }
        cluster = next;
    }
    Ok(result)
}

/// Освобождает всю цепочку начиная с `start`.
fn free_chain(disk: &Mutex<Box<dyn DiskIo>>, geo: &Fat32Geo, start: u32) -> Result<(), FsError> {
    for cluster in chain(disk, geo, start)? {
        fat_set(disk, geo, cluster, FAT_FREE)?;
    }
    Ok(())
}

/// Выделяет свободный кластер и (опционально) привязывает его к `prev`.
fn alloc_cluster(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    prev: Option<u32>,
) -> Result<u32, FsError> {
    let end = geo.total_clusters + 2;
    for cluster in 2..end {
        if fat_get(disk, geo, cluster)? == FAT_FREE {
            fat_set(disk, geo, cluster, FAT_EOC)?;
            if let Some(previous) = prev {
                fat_set(disk, geo, previous, cluster)?;
            }
            let _ = update_fsinfo(disk, geo, cluster);
            return Ok(cluster);
        }
    }
    Err(FsError::DiskFull)
}

/// FSInfo: подсказка ОС о числе свободных кластеров и следующем свободном.
/// Ошибки игнорируются — это лишь оптимизация, том остаётся консистентным.
fn update_fsinfo(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    next_free: u32,
) -> Result<(), FsError> {
    if geo.fs_info_sector == 0 || geo.fs_info_sector == 0xFFFF {
        return Ok(());
    }
    let mut buf = [0u8; SECTOR_SIZE];
    read_sector(disk, geo.fs_info_sector as u64, &mut buf)?;
    if u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != 0x4161_5252 {
        return Ok(());
    }
    if u32::from_le_bytes([buf[484], buf[485], buf[486], buf[487]]) != 0x6141_7272 {
        return Ok(());
    }

    let free = u32::from_le_bytes([buf[488], buf[489], buf[490], buf[491]]);
    if free != 0xFFFF_FFFF && free > 0 {
        buf[488..492].copy_from_slice(&(free - 1).to_le_bytes());
    }
    buf[492..496].copy_from_slice(&next_free.to_le_bytes());
    write_sector(disk, geo.fs_info_sector as u64, &buf)
}

/// Читает всю цепочку файла в память (без учёта `file_size`).
fn read_chain(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    start: u32,
) -> Result<Vec<u8>, FsError> {
    let clusters = chain(disk, geo, start)?;
    let cluster_size = geo.cluster_size as usize;
    let mut data = Vec::with_capacity(clusters.len() * cluster_size);
    let mut buf = vec![0u8; cluster_size];

    for cluster in clusters {
        let sector = geo.cluster_to_sector(cluster).ok_or(FsError::InvalidPath)?;
        for index in 0..geo.sectors_per_cluster as usize {
            let begin = index * SECTOR_SIZE;
            read_sector(
                disk,
                sector + index as u64,
                &mut buf[begin..begin + SECTOR_SIZE],
            )?;
        }
        data.extend_from_slice(&buf);
    }
    Ok(data)
}

/// Пишет `data` в кластер по внутреннему смещению (read-modify-write для
/// частичных секторов).
fn write_cluster_range(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    cluster: u32,
    within: usize,
    data: &[u8],
) -> Result<(), FsError> {
    let base = geo.cluster_to_sector(cluster).ok_or(FsError::InvalidPath)?;
    let max_sector = geo.sectors_per_cluster as usize;

    let mut sector_index = within / SECTOR_SIZE;
    let mut sector_offset = within % SECTOR_SIZE;
    let mut done = 0usize;

    while done < data.len() {
        if sector_index >= max_sector {
            return Err(FsError::IoError);
        }
        let take = (SECTOR_SIZE - sector_offset).min(data.len() - done);
        let sector = base + sector_index as u64;
        if sector_offset == 0 && take == SECTOR_SIZE {
            write_sector(disk, sector, &data[done..done + take])?;
        } else {
            let mut buf = [0u8; SECTOR_SIZE];
            read_sector(disk, sector, &mut buf)?;
            buf[sector_offset..sector_offset + take].copy_from_slice(&data[done..done + take]);
            write_sector(disk, sector, &buf)?;
        }
        done += take;
        sector_index += 1;
        sector_offset = 0;
    }
    Ok(())
}

/// Пишет `data` по смещению `offset` в файл, начиная с `*start`.
///
/// Цепочка продлевается аллокацией кластеров; `*start` обновляется, если файл
/// был пустым. Возвращает новый размер файла (offset + data.len()).
pub fn write_range(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    start: &mut u32,
    offset: u64,
    data: &[u8],
) -> Result<u64, FsError> {
    if data.is_empty() {
        return Ok(offset);
    }
    let cluster_size = geo.cluster_size as u64;

    let mut cluster = if *start < 2 {
        let first = alloc_cluster(disk, geo, None)?;
        *start = first;
        first
    } else {
        *start
    };

    // Дойти до кластера, содержащего `offset`.
    let mut index = 0u64;
    while index < offset / cluster_size {
        let next = fat_get(disk, geo, cluster)?;
        cluster = if next < 2 || is_end_of_chain(next) {
            alloc_cluster(disk, geo, Some(cluster))?
        } else {
            next
        };
        index += 1;
    }

    let mut done = 0usize;
    let mut position = offset;
    while done < data.len() {
        let within = (position % cluster_size) as usize;
        let chunk = (cluster_size as usize - within).min(data.len() - done);
        write_cluster_range(disk, geo, cluster, within, &data[done..done + chunk])?;
        done += chunk;
        position += chunk as u64;

        if done < data.len() {
            let next = fat_get(disk, geo, cluster)?;
            cluster = if next < 2 || is_end_of_chain(next) {
                alloc_cluster(disk, geo, Some(cluster))?
            } else {
                next
            };
        }
    }
    Ok(offset + data.len() as u64)
}

// ==========================================
// Каталоги
// ==========================================

/// Запись каталога вместе с её адресом на диске: нужна для обновления
/// `first_cluster`/`file_size` без повторного поиска.
#[derive(Clone, Copy)]
pub struct DirSlot {
    pub entry: FatDirEntry,
    pub sector: u64,
    pub offset: usize,
}

fn entry_bytes(entry: &FatDirEntry) -> [u8; 32] {
    unsafe { core::ptr::read(entry as *const FatDirEntry as *const [u8; 32]) }
}

/// Читает каталог (всю цепочку) и возвращает записи с именами.
///
/// Поддержаны LFN (Long File Name): цепочка записей с атрибутом 0x0F идёт
/// перед 8.3-записью в обратном порядке.
fn read_dir(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    dir_cluster: u32,
) -> Result<Vec<(DirSlot, String)>, FsError> {
    let mut result = Vec::new();
    let mut lfn: Vec<[u16; 13]> = Vec::new();

    for cluster in chain(disk, geo, dir_cluster)? {
        let base = geo.cluster_to_sector(cluster).ok_or(FsError::InvalidPath)?;
        for index in 0..geo.sectors_per_cluster as u64 {
            let sector = base + index;
            let mut buf = [0u8; SECTOR_SIZE];
            read_sector(disk, sector, &mut buf)?;

            for offset in (0..SECTOR_SIZE).step_by(32) {
                let entry: FatDirEntry =
                    unsafe { core::ptr::read(buf[offset..].as_ptr() as *const FatDirEntry) };

                if entry.is_end() {
                    return Ok(result);
                }
                if entry.is_free() {
                    lfn.clear();
                    continue;
                }
                if entry.is_long_name() {
                    let mut part = [0u16; 13];
                    let mut i = 0;
                    // Поля имени LFN — UTF-16LE: 5 символов (1..11),
                    // 6 символов (14..26), 2 символа (28..32).
                    for (from, to) in [(1usize, 11usize), (14, 26), (28, 32)] {
                        for position in (from..to).step_by(2) {
                            if i < 13 {
                                part[i] = u16::from_le_bytes([
                                    buf[offset + position],
                                    buf[offset + position + 1],
                                ]);
                                i += 1;
                            }
                        }
                    }
                    lfn.push(part);
                    continue;
                }

                let name = if lfn.is_empty() {
                    entry.get_name()
                } else {
                    let mut text = String::new();
                    for part in lfn.iter().rev() {
                        for &c in part.iter() {
                            // 0x0000 — терминатор, 0xFFFF — padding.
                            if c == 0 || c == 0xFFFF {
                                continue;
                            }
                            text.push(char::from_u32(c as u32).unwrap_or('?'));
                        }
                    }
                    if text.is_empty() {
                        entry.get_name()
                    } else {
                        text
                    }
                };
                lfn.clear();

                // 0x08 = volume label: в листинг не попадает.
                if entry.attributes & FatDirEntry::ATTR_VOLUME_ID == 0 {
                    result.push((DirSlot { entry, sector, offset }, name));
                }
            }
        }
    }
    Ok(result)
}

fn find_in_dir(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    dir_cluster: u32,
    name: &str,
) -> Result<Option<DirSlot>, FsError> {
    for (slot, entry_name) in read_dir(disk, geo, dir_cluster)? {
        if entry_name.eq_ignore_ascii_case(name) {
            return Ok(Some(slot));
        }
    }
    Ok(None)
}

/// Ищет путь и возвращает запись с адресом. `/` возвращает корень.
fn find_path(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    path: &str,
) -> Result<Option<DirSlot>, FsError> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Ok(Some(DirSlot {
            entry: FatDirEntry {
                name: [b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                attributes: FatDirEntry::ATTR_DIRECTORY,
                reserved: 0,
                creation_time_tenths: 0,
                creation_time: 0,
                creation_date: 0,
                last_access_date: 0,
                first_cluster_high: (geo.root_cluster >> 16) as u16,
                write_time: 0,
                write_date: 0,
                first_cluster_low: (geo.root_cluster & 0xFFFF) as u16,
                file_size: 0,
            },
            sector: 0,
            offset: 0,
        }));
    }

    let mut current = geo.root_cluster;
    for (index, part) in parts.iter().enumerate() {
        let Some(slot) = find_in_dir(disk, geo, current, part)? else {
            return Ok(None);
        };
        if index + 1 == parts.len() {
            return Ok(Some(slot));
        }
        if !slot.entry.is_directory() {
            return Err(FsError::NotADirectory);
        }
        current = slot.entry.get_cluster();
    }
    Ok(None)
}

/// Кластер каталога, в котором лежит `path`, и имя файла (для создания).
fn parent_cluster(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    path: &str,
) -> Result<(u32, String), FsError> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((name, dirs)) = parts.split_last() else {
        return Err(FsError::InvalidPath);
    };

    let mut current = geo.root_cluster;
    for part in dirs {
        let Some(slot) = find_in_dir(disk, geo, current, part)? else {
            return Err(FsError::NotFound);
        };
        if !slot.entry.is_directory() {
            return Err(FsError::NotADirectory);
        }
        current = slot.entry.get_cluster();
    }
    Ok((current, String::from(*name)))
}

/// Преобразование имени в 8.3 (верхний регистр, выравнивание пробелами).
fn to_83(name: &str) -> Option<[u8; 11]> {
    let upper = name.to_ascii_uppercase();
    let (base, ext) = match upper.rfind('.') {
        Some(position) => (&upper[..position], &upper[position + 1..]),
        None => (&upper[..], ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return None;
    }

    let mut out = [b' '; 11];
    for (index, byte) in base.bytes().enumerate() {
        if !is_valid_83(byte) {
            return None;
        }
        out[index] = byte;
    }
    for (index, byte) in ext.bytes().enumerate() {
        if !is_valid_83(byte) {
            return None;
        }
        out[8 + index] = byte;
    }
    Some(out)
}

fn is_valid_83(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'()-@^_`{}~".contains(&byte)
}

/// Дата/время в формате FAT (DOS): дата с 1980 года, время с шагом 2 секунды.
fn dos_now() -> (u16, u16) {
    let info = crate::timer::clock_info();
    let Some(rtc) = info.rtc else {
        return (0, 0);
    };
    let year = (rtc.year as u16).clamp(1980, 2107) - 1980;
    let date = (year << 9) | ((rtc.month as u16) << 5) | rtc.day as u16;
    let time =
        ((rtc.hour as u16) << 11) | ((rtc.minute as u16) << 5) | ((rtc.second as u16) / 2);
    (date, time)
}

/// Обновляет `first_cluster`/`file_size` записи каталога.
fn set_entry(
    disk: &Mutex<Box<dyn DiskIo>>,
    slot: &DirSlot,
    cluster: u32,
    size: u32,
) -> Result<(), FsError> {
    let mut buf = [0u8; SECTOR_SIZE];
    read_sector(disk, slot.sector, &mut buf)?;
    let mut entry: FatDirEntry =
        unsafe { core::ptr::read(buf[slot.offset..].as_ptr() as *const FatDirEntry) };
    entry.set_cluster(cluster);
    entry.file_size = size;
    let (date, time) = dos_now();
    entry.write_date = date;
    entry.write_time = time;
    buf[slot.offset..slot.offset + 32].copy_from_slice(&entry_bytes(&entry));
    write_sector(disk, slot.sector, &buf)
}

/// Находит свободный слот в каталоге; при необходимости добавляет кластер.
fn find_free_slot(
    disk: &Mutex<Box<dyn DiskIo>>,
    geo: &Fat32Geo,
    dir_cluster: u32,
) -> Result<(u64, usize), FsError> {
    let clusters = chain(disk, geo, dir_cluster)?;
    let mut last = dir_cluster;

    for cluster in clusters {
        last = cluster;
        let base = geo.cluster_to_sector(cluster).ok_or(FsError::InvalidPath)?;
        for index in 0..geo.sectors_per_cluster as u64 {
            let mut buf = [0u8; SECTOR_SIZE];
            read_sector(disk, base + index, &mut buf)?;
            for offset in (0..SECTOR_SIZE).step_by(32) {
                if buf[offset] == 0x00 || buf[offset] == 0xE5 {
                    return Ok((base + index, offset));
                }
            }
        }
    }

    // Каталог заполнен — добавляем кластер и обнуляем его.
    let new = alloc_cluster(disk, geo, Some(last))?;
    let base = geo.cluster_to_sector(new).ok_or(FsError::InvalidPath)?;
    let zeros = [0u8; SECTOR_SIZE];
    for index in 0..geo.sectors_per_cluster as u64 {
        write_sector(disk, base + index, &zeros)?;
    }
    Ok((base, 0))
}

/// FAT32-том: геометрия + диск. Один экземпляр на монтирование.
pub struct Fat32Fs {
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    geo: Fat32Geo,
    mounted: bool,
    read_only: bool,
}

impl Fat32Fs {
    /// Только чтение (поведение по умолчанию для пользовательских томов).
    pub fn new(disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        Self::with_mode(disk, true)
    }

    /// Чтение и запись. Включается явно (boot-log, тестовые образы).
    pub fn new_rw(disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        Self::with_mode(disk, false)
    }

    fn with_mode(disk: Box<dyn DiskIo>, read_only: bool) -> Result<Self, FsError> {
        let shared = Arc::new(Mutex::new(disk));

        let mut buf = [0u8; SECTOR_SIZE];
        read_sector(&shared, 0, &mut buf)?;
        let boot: Fat32BootSector = unsafe { core::ptr::read(buf.as_ptr() as *const _) };

        // FAT32: fat_size_16 == 0 и root_entry_count == 0 (корневой каталог —
        // обычная цепочка кластеров, а не фиксированная область). Если это не
        // так, перед нами FAT12/16 и раскладка data-области другая.
        // bytes_per_sector может быть 512 или 4096 (4Kn): FS-слой работает
        // в 512-байтных секторах, поэтому геометрию масштабируем.
        let sector_shift = match boot.bytes_per_sector {
            512 => 1u64,
            4096 => 8u64,
            _ => return Err(FsError::InvalidPath),
        };
        if boot.fat_size_16 != 0 || boot.root_entry_count != 0 {
            return Err(FsError::InvalidPath);
        }
        if boot.sectors_per_cluster == 0 || !boot.sectors_per_cluster.is_power_of_two() {
            return Err(FsError::InvalidPath);
        }
        // Повреждённый BPB не должен приводить к делению на ноль или чтению
        // за границей тома: проверяем ключевые поля геометрии.
        if boot.reserved_sectors == 0 || boot.fat_size_32 == 0 {
            return Err(FsError::InvalidPath);
        }
        if boot.fat_count == 0 || boot.fat_count > 4 {
            return Err(FsError::InvalidPath);
        }

        let spc_512 = (boot.sectors_per_cluster as u64)
            .checked_mul(sector_shift)
            .ok_or(FsError::InvalidPath)?;
        if spc_512 > u8::MAX as u64 {
            return Err(FsError::InvalidPath);
        }

        let fat_start = (boot.reserved_sectors as u64)
            .checked_mul(sector_shift)
            .ok_or(FsError::InvalidPath)?;
        let fat_size = (boot.fat_size_32 as u64)
            .checked_mul(sector_shift)
            .ok_or(FsError::InvalidPath)?;
        let data_start = fat_start + (boot.fat_count as u64 * fat_size)
            + ((boot.root_entry_count as u64 * 32 + 511) / 512);
        let cluster_size = (512u64 * spc_512) as u32;
        let total_sectors = if boot.total_sectors_16 != 0 {
            boot.total_sectors_16 as u64
        } else {
            boot.total_sectors_32 as u64
        }
        .checked_mul(sector_shift)
        .ok_or(FsError::InvalidPath)?;
        let data_sectors = total_sectors.saturating_sub(data_start);
        let total_clusters = (data_sectors / spc_512) as u32;
        if total_clusters < 2 {
            return Err(FsError::InvalidPath);
        }

        let fs_info_sector = (boot.fs_info_sector as u64)
            .checked_mul(sector_shift)
            .filter(|v| *v <= u16::MAX as u64)
            .map(|v| v as u16)
            .unwrap_or(0);

        Ok(Self {
            disk: shared,
            geo: Fat32Geo {
                fat_start_sector: fat_start,
                fat_count: boot.fat_count,
                fat_size: fat_size as u32,
                data_start_sector: data_start,
                sectors_per_cluster: spc_512 as u8,
                cluster_size,
                root_cluster: boot.root_cluster,
                fs_info_sector,
                total_clusters,
                volume_label: boot.volume_label,
            },
            mounted: false,
            read_only,
        })
    }

    pub fn geo(&self) -> &Fat32Geo {
        &self.geo
    }

    /// Метка тома из BPB (11 байт, выровнена пробелами).
    pub fn label(&self) -> [u8; 11] {
        self.geo.volume_label
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn cluster_size(&self) -> u32 {
        self.geo.cluster_size
    }

    /// Поиск файла: `(start_cluster, size, dir_slot)`.
    pub fn lookup(&self, path: &str) -> Result<Option<(u32, u32, DirSlot)>, FsError> {
        match find_path(&self.disk, &self.geo, path)? {
            Some(slot) if slot.entry.is_file() => Ok(Some((
                slot.entry.get_cluster(),
                slot.entry.file_size,
                slot,
            ))),
            Some(_) => Err(FsError::IsADirectory),
            None => Ok(None),
        }
    }

    /// Цепочка кластеров файла.
    pub fn clusters(&self, start: u32) -> Result<Vec<u32>, FsError> {
        chain(&self.disk, &self.geo, start)
    }

    /// Запись в файл по смещению. Размер в каталоге обновляет вызывающий
    /// через `publish_size` — так данные попадают на диск раньше метаданных,
    /// и при обрыве питания файл не «врёт» о содержимом.
    pub fn write_at(&mut self, start: &mut u32, offset: u64, data: &[u8]) -> Result<u64, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        write_range(&self.disk, &self.geo, start, offset, data)
    }

    /// Публикует размер файла в записи каталога.
    pub fn publish_size(&mut self, slot: &DirSlot, start: u32, size: u32) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        set_entry(&self.disk, slot, start, size)
    }

    /// Усекает файл до нуля (освобождая цепочку).
    pub fn truncate(&mut self, path: &str) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        if !slot.entry.is_file() {
            return Err(FsError::IsADirectory);
        }
        free_chain(&self.disk, &self.geo, slot.entry.get_cluster())?;
        set_entry(&self.disk, &slot, 0, 0)
    }
}

/// FAT32 файл. Данные держатся в памяти (как и раньше), но запись идёт
/// сквозь на диск: `write` обновляет и кэш, и цепочку кластеров.
pub struct Fat32File {
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    geo: Fat32Geo,
    slot: DirSlot,
    start_cluster: u32,
    size: u64,
    position: u64,
    data: Vec<u8>,
    read_only: bool,
}

impl File for Fat32File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() {
            return Ok(0);
        }
        let to_read = buf.len().min(self.data.len() - pos);
        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let end = self.position as usize + buf.len();
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[self.position as usize..end].copy_from_slice(buf);

        let new_end = write_range(
            &self.disk,
            &self.geo,
            &mut self.start_cluster,
            self.position,
            buf,
        )?;
        if new_end > self.size {
            self.size = new_end;
        }
        set_entry(
            &self.disk,
            &self.slot,
            self.start_cluster,
            self.size.min(u32::MAX as u64) as u32,
        )?;
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::Current(o) => self.position as i64 + o,
            SeekFrom::End(o) => self.data.len() as i64 + o,
        };
        if new < 0 {
            return Err(FsError::InvalidPath);
        }
        self.position = new as u64;
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

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        if slot.entry.is_directory() {
            return Err(FsError::IsADirectory);
        }
        if !slot.entry.is_file() {
            return Err(FsError::NotFound);
        }

        let start_cluster = slot.entry.get_cluster();
        let size = slot.entry.file_size as usize;
        let mut data = read_chain(&self.disk, &self.geo, start_cluster)?;
        data.truncate(size.min(data.len()));

        Ok(Box::new(Fat32File {
            disk: self.disk.clone(),
            geo: self.geo,
            slot,
            start_cluster,
            size: size as u64,
            position: 0,
            data,
            read_only: self.read_only,
        }))
    }

    fn create(&mut self, path: &str) -> Result<Box<dyn File>, FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let (dir_cluster, name) = parent_cluster(&self.disk, &self.geo, path)?;
        let name83 = to_83(&name).ok_or(FsError::NotSupported)?;

        let slot = match find_in_dir(&self.disk, &self.geo, dir_cluster, &name)? {
            Some(slot) => {
                if slot.entry.is_directory() {
                    return Err(FsError::IsADirectory);
                }
                // Пересоздание: освобождаем старую цепочку и обнуляем запись.
                free_chain(&self.disk, &self.geo, slot.entry.get_cluster())?;
                set_entry(&self.disk, &slot, 0, 0)?;
                slot
            }
            None => {
                let (sector, offset) = find_free_slot(&self.disk, &self.geo, dir_cluster)?;
                let (date, time) = dos_now();
                let entry = FatDirEntry {
                    name: name83,
                    attributes: 0x20, // archive
                    reserved: 0,
                    creation_time_tenths: 0,
                    creation_time: time,
                    creation_date: date,
                    last_access_date: date,
                    first_cluster_high: 0,
                    write_time: time,
                    write_date: date,
                    first_cluster_low: 0,
                    file_size: 0,
                };
                let mut buf = [0u8; SECTOR_SIZE];
                read_sector(&self.disk, sector, &mut buf)?;
                buf[offset..offset + 32].copy_from_slice(&entry_bytes(&entry));
                write_sector(&self.disk, sector, &buf)?;
                DirSlot {
                    entry,
                    sector,
                    offset,
                }
            }
        };

        Ok(Box::new(Fat32File {
            disk: self.disk.clone(),
            geo: self.geo,
            slot,
            start_cluster: 0,
            size: 0,
            position: 0,
            data: Vec::new(),
            read_only: false,
        }))
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        if slot.entry.is_directory() {
            return Err(FsError::IsADirectory);
        }

        free_chain(&self.disk, &self.geo, slot.entry.get_cluster())?;

        // 0xE5 = удалённая запись (имя сохраняется для восстановления).
        let mut buf = [0u8; SECTOR_SIZE];
        read_sector(&self.disk, slot.sector, &mut buf)?;
        buf[slot.offset] = 0xE5;
        write_sector(&self.disk, slot.sector, &buf)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }

        let (dir_cluster, name) = parent_cluster(&self.disk, &self.geo, path)?;
        let name83 = to_83(&name).ok_or(FsError::NotSupported)?;

        // Уже существует (файл или каталог) — это не ошибка драйвера, но и не
        // повод перезаписывать чужую цепочку кластеров.
        if find_in_dir(&self.disk, &self.geo, dir_cluster, &name)?.is_some() {
            return Err(FsError::AlreadyExists);
        }

        // Каталог обязан занимать хотя бы один кластер: в нём живут записи
        // `.` и `..`, без них том считается повреждённым (chkdsk/fsck).
        let new_cluster = alloc_cluster(&self.disk, &self.geo, None)?;
        let base = self
            .geo
            .cluster_to_sector(new_cluster)
            .ok_or(FsError::InvalidPath)?;

        let zeros = [0u8; SECTOR_SIZE];
        for index in 0..self.geo.sectors_per_cluster as u64 {
            write_sector(&self.disk, base + index, &zeros)?;
        }

        let (date, time) = dos_now();
        let make_entry = |raw_name: [u8; 11], cluster: u32| FatDirEntry {
            name: raw_name,
            attributes: FatDirEntry::ATTR_DIRECTORY,
            reserved: 0,
            creation_time_tenths: 0,
            creation_time: time,
            creation_date: date,
            last_access_date: date,
            first_cluster_high: (cluster >> 16) as u16,
            write_time: time,
            write_date: date,
            first_cluster_low: (cluster & 0xFFFF) as u16,
            file_size: 0,
        };

        // `.` указывает на сам каталог, `..` — на родителя. Для каталога в
        // корне FAT32 `..` обязан содержать кластер 0, а не номер корневого
        // кластера: иначе том не проходит проверку консистентности.
        let dot = make_entry(*b".          ", new_cluster);
        let parent_for_dotdot = if dir_cluster == self.geo.root_cluster {
            0
        } else {
            dir_cluster
        };
        let dotdot = make_entry(*b"..         ", parent_for_dotdot);

        let mut buf = [0u8; SECTOR_SIZE];
        buf[0..32].copy_from_slice(&entry_bytes(&dot));
        buf[32..64].copy_from_slice(&entry_bytes(&dotdot));
        write_sector(&self.disk, base, &buf)?;

        // Запись о каталоге в родительском каталоге.
        let (sector, offset) = find_free_slot(&self.disk, &self.geo, dir_cluster)?;
        let entry = make_entry(name83, new_cluster);
        let mut parent_buf = [0u8; SECTOR_SIZE];
        read_sector(&self.disk, sector, &mut parent_buf)?;
        parent_buf[offset..offset + 32].copy_from_slice(&entry_bytes(&entry));
        write_sector(&self.disk, sector, &parent_buf)
    }

    fn rmdir(&mut self, path: &str) -> Result<(), FsError> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }

        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        if !slot.entry.is_directory() {
            return Err(FsError::NotADirectory);
        }

        let cluster = slot.entry.get_cluster();
        // Корневой каталог удалить нельзя: у него нет записи в родителе, и
        // освобождение его цепочки сделало бы том нечитаемым.
        if cluster == self.geo.root_cluster || cluster < 2 {
            return Err(FsError::PermissionDenied);
        }

        // Каталог удаляется только пустым. Записи `.` и `..` пустотой не
        // считаются — иначе `rmdir` сносил бы непустые каталоги, оставляя
        // их содержимое в виде потерянных кластеров.
        for (entry, name) in read_dir(&self.disk, &self.geo, cluster)? {
            if name == "." || name == ".." || name.is_empty() {
                continue;
            }
            let _ = entry;
            return Err(FsError::NotEmpty);
        }

        free_chain(&self.disk, &self.geo, cluster)?;

        // 0xE5 = удалённая запись (имя сохраняется для восстановления).
        let mut buf = [0u8; SECTOR_SIZE];
        read_sector(&self.disk, slot.sector, &mut buf)?;
        buf[slot.offset] = 0xE5;
        write_sector(&self.disk, slot.sector, &buf)
    }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        let cluster = if path == "/" || path.is_empty() {
            self.geo.root_cluster
        } else {
            if !slot.entry.is_directory() {
                return Err(FsError::NotADirectory);
            }
            slot.entry.get_cluster()
        };

        let mut result = Vec::new();
        for (entry, name) in read_dir(&self.disk, &self.geo, cluster)? {
            if name == "." || name == ".." || name.is_empty() {
                continue;
            }
            let file_type = if entry.entry.is_directory() {
                FileType::Directory
            } else {
                FileType::File
            };
            let perms = if entry.entry.is_directory() { 0o755 } else { 0o644 };
            result.push(DirEntry {
                name,
                file_type,
                size: entry.entry.file_size as usize,
                permissions: Permissions::new(perms),
                uid: 0,
                gid: 0,
            });
        }
        Ok(result)
    }

    fn exists(&self, path: &str) -> bool {
        matches!(find_path(&self.disk, &self.geo, path), Ok(Some(_)))
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let Some(slot) = find_path(&self.disk, &self.geo, path)? else {
            return Err(FsError::NotFound);
        };
        // Имя берём из пути: у записи может быть LFN, а get_name() вернёт 8.3.
        let name = String::from(path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/"));
        let file_type = if slot.entry.is_directory() {
            FileType::Directory
        } else {
            FileType::File
        };
        let perms = if slot.entry.is_directory() { 0o755 } else { 0o644 };

        Ok(FileMetadata {
            name,
            file_type,
            size: slot.entry.file_size as usize,
            permissions: Permissions::new(perms),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
        })
    }
}
