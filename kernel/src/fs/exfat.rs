// fs/exfat.rs — exFAT, только чтение.
//
// Закрывает типичный внешний HDD на 2–4 ТБ, который сейчас виден как
// `exfat` и не монтируется. Реализован минимум, нужный для чтения:
//
//   * boot sector: геометрия кластеров, FAT, cluster heap;
//   * FAT-цепочки и contiguous-файлы (`NoFatChain`, типичный случай для
//     больших видеофайлов);
//   * каталоги: entry set 0x85 (File) + 0xC0 (Stream) + 0xC1 (Name);
//   * имена в UTF-16.
//
// Чего нет и что для чтения не требуется: allocation bitmap, upcase-таблица
// (hash имени не сверяется), timestamps, запись.
//
// Как и в NTFS, файл читается по требованию: `cat` 2-гигабайтного файла не
// пытается занять его целиком в куче.

use super::disk::DiskIo;
use super::vfs::{
    DirEntry, File, FileMetadata, FileSystem, FileType, FsError, Permissions, SeekFrom,
};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

const EXFAT_NAME: &[u8; 8] = b"EXFAT   ";

/// Максимум, который готовы прочитать из одного каталога.
const MAX_DIR_BYTES: usize = 64 * 1024 * 1024;

/// Признак конца цепочки кластеров в FAT.
const FAT_EOC: u32 = 0xFFFF_FFF8;
/// Кластер помечен как сбойный.
const FAT_BAD: u32 = 0xFFFF_FFF7;

const ATTR_DIRECTORY: u16 = 0x0010;

/// Геометрия тома из boot sector.
#[derive(Clone, Copy)]
struct ExfatBoot {
    bytes_per_sector: u32,
    sectors_per_cluster: u32,
    fat_offset: u32,
    fat_length: u32,
    cluster_heap_offset: u32,
    cluster_count: u32,
    root_cluster: u32,
    number_of_fats: u8,
}

impl ExfatBoot {
    fn parse(s: &[u8]) -> Result<Self, FsError> {
        if s.len() < 512 || &s[3..11] != EXFAT_NAME {
            return Err(FsError::InvalidPath);
        }

        let bps_shift = s[108] as u32;
        let spc_shift = s[109] as u32;
        // Спецификация: 512…4096 байт на сектор и не больше 32 МиБ на кластер.
        if !(9..=12).contains(&bps_shift) || spc_shift > 25 {
            return Err(FsError::InvalidPath);
        }
        let bytes_per_sector = 1u32 << bps_shift;
        let sectors_per_cluster = 1u32 << spc_shift;
        let cluster_size = bytes_per_sector as u64 * sectors_per_cluster as u64;
        if cluster_size > 32 * 1024 * 1024 {
            return Err(FsError::InvalidPath);
        }

        let fat_offset = rd_u32(s, 80);
        let fat_length = rd_u32(s, 84);
        let cluster_heap_offset = rd_u32(s, 88);
        let cluster_count = rd_u32(s, 92);
        let root_cluster = rd_u32(s, 96);
        let number_of_fats = s[110];

        if fat_offset == 0 || fat_length == 0 || cluster_count == 0 {
            return Err(FsError::InvalidPath);
        }
        if number_of_fats != 1 && number_of_fats != 2 {
            return Err(FsError::InvalidPath);
        }
        // Кластеры нумеруются с 2: 0 и 1 — служебные записи FAT.
        if root_cluster < 2 || root_cluster >= cluster_count + 2 {
            return Err(FsError::InvalidPath);
        }
        let heap_start = fat_offset as u64 + fat_length as u64 * number_of_fats as u64;
        if cluster_heap_offset < heap_start as u32 {
            return Err(FsError::InvalidPath);
        }

        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            fat_offset,
            fat_length,
            cluster_heap_offset,
            cluster_count,
            root_cluster,
            number_of_fats,
        })
    }

    fn cluster_size(&self) -> u64 {
        self.bytes_per_sector as u64 * self.sectors_per_cluster as u64
    }

    /// LBA кластера в 512-байтных единицах block layer (не в секторах тома:
    /// на 4Kn-носителе это разные вещи).
    fn cluster_lba(&self, cluster: u32) -> u64 {
        let sectors_512 = (self.bytes_per_sector / 512) as u64;
        (self.cluster_heap_offset as u64 + (cluster - 2) as u64 * self.sectors_per_cluster as u64)
            * sectors_512
    }

}

fn rd_u16(s: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([s[off], s[off + 1]])
}

fn rd_u32(s: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([s[off], s[off + 1], s[off + 2], s[off + 3]])
}

fn rd_u64(s: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        s[off],
        s[off + 1],
        s[off + 2],
        s[off + 3],
        s[off + 4],
        s[off + 5],
        s[off + 6],
        s[off + 7],
    ])
}

/// Значение записи FAT для кластера: номер следующего либо признак конца.
fn fat_entry(disk: &Mutex<Box<dyn DiskIo>>, boot: &ExfatBoot, cluster: u32) -> Result<u32, FsError> {
    if cluster >= boot.cluster_count + 2 {
        return Err(FsError::IoError);
    }
    // Первая FAT для чтения достаточна: копии синхронизируются драйвером
    // записи, а нам нужна только одна.
    let byte = boot.fat_offset as u64 * boot.bytes_per_sector as u64 + cluster as u64 * 4;
    let sector = byte / 512;
    let offset = (byte % 512) as usize;

    let mut buf = [0u8; 512];
    disk.lock().read(sector, &mut buf).map_err(|_| FsError::IoError)?;
    Ok(rd_u32(&buf, offset))
}

fn next_cluster(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    cluster: u32,
) -> Result<Option<u32>, FsError> {
    let next = fat_entry(disk, boot, cluster)?;
    if next >= FAT_EOC {
        return Ok(None);
    }
    if next == FAT_BAD || next < 2 || next >= boot.cluster_count + 2 {
        return Ok(None);
    }
    Ok(Some(next))
}

/// Список кластеров цепочки, но не больше `max_clusters`.
fn chain(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    first: u32,
    max_clusters: u32,
) -> Result<Vec<u32>, FsError> {
    let mut out = Vec::new();
    let mut cur = first;
    while out.len() < max_clusters as usize {
        if cur < 2 || cur >= boot.cluster_count + 2 {
            break;
        }
        out.push(cur);
        match next_cluster(disk, boot, cur)? {
            Some(next) => cur = next,
            None => break,
        }
    }
    if out.is_empty() {
        return Err(FsError::IoError);
    }
    Ok(out)
}

/// Прочитать содержимое цепочки каталога (не больше `max_bytes`).
fn read_chain(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    first: u32,
    max_bytes: usize,
) -> Result<Vec<u8>, FsError> {
    let cluster_size = boot.cluster_size() as usize;
    if cluster_size == 0 {
        return Err(FsError::IoError);
    }
    let max_clusters = ((max_bytes / cluster_size) as u32).max(1);
    let clusters = chain(disk, boot, first, max_clusters)?;

    let mut out = vec![0u8; clusters.len() * cluster_size];
    let mut disk = disk.lock();
    for (i, &cluster) in clusters.iter().enumerate() {
        let lba = boot.cluster_lba(cluster);
        let mut buf = vec![0u8; cluster_size];
        // `read` работает 512-байтными единицами, поэтому кластер любого
        // размера читается одним вызовом по LBA из cluster heap.
        disk.read(lba, &mut buf).map_err(|_| FsError::IoError)?;
        out[i * cluster_size..(i + 1) * cluster_size].copy_from_slice(&buf);
    }
    Ok(out)
}

/// Запись каталога, собранная из entry set.
#[derive(Clone)]
struct EntryInfo {
    name: String,
    is_dir: bool,
    first_cluster: u32,
    size: u64,
    /// `NoFatChain`: кластеры идут подряд, FAT для файла не заполнена.
    no_fat_chain: bool,
}

fn utf16_to_string(units: &[u16], len: usize) -> String {
    let mut s = String::new();
    let n = len.min(units.len());
    let mut i = 0;
    while i < n {
        let u = units[i];
        if (0xD800..0xDC00).contains(&u) && i + 1 < n {
            let lo = units[i + 1];
            if (0xDC00..0xE000).contains(&lo) {
                let cp = 0x10000 + (((u - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
                s.push(char::from_u32(cp).unwrap_or('?'));
                i += 2;
                continue;
            }
        }
        s.push(char::from_u32(u as u32).unwrap_or('?'));
        i += 1;
    }
    s
}

/// Разобрать каталог в список записей.
fn list_entries(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    dir_cluster: u32,
) -> Result<Vec<EntryInfo>, FsError> {
    let data = read_chain(disk, boot, dir_cluster, MAX_DIR_BYTES)?;
    let mut out = Vec::new();

    let mut i = 0usize;
    while i + 32 <= data.len() {
        let etype = data[i];
        // 0x00 — дальше записей нет, остальное — мусор после удаления.
        if etype == 0x00 {
            break;
        }
        // bit7 = 1 — запись используется; иначе удалена или зарезервирована.
        if etype & 0x80 == 0 {
            i += 32;
            continue;
        }

        if etype != 0x85 {
            i += 32;
            continue;
        }

        let secondary = data[i + 1] as usize;
        let attributes = rd_u16(&data, i + 4);
        let mut info = EntryInfo {
            name: String::new(),
            is_dir: attributes & ATTR_DIRECTORY != 0,
            first_cluster: 0,
            size: 0,
            no_fat_chain: false,
        };
        let mut name_len = 0usize;
        let mut units: Vec<u16> = Vec::new();

        for j in 1..=secondary {
            let off = i + j * 32;
            if off + 32 > data.len() {
                break;
            }
            let sec = &data[off..off + 32];
            match sec[0] {
                // Stream Extension: размер и первый кластер.
                0xC0 => {
                    info.no_fat_chain = sec[1] & 0x02 != 0;
                    name_len = sec[3] as usize;
                    info.first_cluster = rd_u32(sec, 20);
                    info.size = rd_u64(sec, 24);
                }
                // File Name: по 15 символов UTF-16.
                0xC1 => {
                    for k in 0..15 {
                        let u = rd_u16(sec, 2 + k * 2);
                        if u != 0 {
                            units.push(u);
                        }
                    }
                }
                _ => {}
            }
        }

        if info.first_cluster < 2 || info.first_cluster >= boot.cluster_count + 2 {
            i += 32 * (1 + secondary);
            continue;
        }

        info.name = utf16_to_string(&units, name_len);
        if !info.name.is_empty() {
            out.push(info);
        }

        // Вся группа entry set уже разобрана — перепрыгиваем её целиком.
        i += 32 * (1 + secondary);
    }

    Ok(out)
}

/// Метка тома exFAT: entry 0x83 в корневом каталоге.
///
/// Нужна для `disks` и `mount ... LABEL=<метка>`: внешние HDD обычно
/// отформатированы именно в exFAT, и без метки их нельзя смонтировать, не
/// зная `sdX`.
pub fn volume_label(disk: &mut dyn DiskIo) -> Option<String> {
    let mut bs = [0u8; 512];
    disk.read(0, &mut bs).ok()?;
    let boot = ExfatBoot::parse(&bs).ok()?;

    let cluster_size = boot.cluster_size() as usize;
    if cluster_size == 0 {
        return None;
    }
    let mut buf = vec![0u8; cluster_size];
    disk.read(boot.cluster_lba(boot.root_cluster), &mut buf).ok()?;

    for entry in buf.chunks(32) {
        if entry.len() < 32 {
            break;
        }
        // 0x00 — дальше записей нет.
        if entry[0] == 0x00 {
            break;
        }
        if entry[0] != 0x83 {
            continue;
        }
        let len = (entry[1] as usize).min(11);
        let mut units = Vec::with_capacity(len);
        for k in 0..len {
            units.push(rd_u16(entry, 2 + k * 2));
        }
        let name = utf16_to_string(&units, len);
        return if name.is_empty() { None } else { Some(name) };
    }
    None
}

fn name_matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a == b
}

/// Найти запись по пути. `/` и пустой путь — корень.
fn find_entry(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    path: &str,
) -> Result<Option<EntryInfo>, FsError> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Ok(None);
    }

    let mut cluster = boot.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let entries = list_entries(disk, boot, cluster)?;
        let Some(entry) = entries.into_iter().find(|e| name_matches(&e.name, part)) else {
            return Ok(None);
        };
        if i + 1 == parts.len() {
            return Ok(Some(entry));
        }
        if !entry.is_dir {
            return Ok(None);
        }
        cluster = entry.first_cluster;
    }
    Ok(None)
}

fn resolve_dir(
    disk: &Mutex<Box<dyn DiskIo>>,
    boot: &ExfatBoot,
    path: &str,
) -> Result<u32, FsError> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Ok(boot.root_cluster);
    }
    match find_entry(disk, boot, path)? {
        Some(e) if e.is_dir => Ok(e.first_cluster),
        Some(_) => Err(FsError::NotADirectory),
        None => Err(FsError::NotFound),
    }
}

fn ro_permissions() -> Permissions {
    Permissions {
        owner_read: true,
        owner_write: false,
        owner_exec: false,
        group_read: true,
        group_write: false,
        group_exec: false,
        other_read: true,
        other_write: false,
        other_exec: false,
    }
}

// ==========================================
// Файловая система
// ==========================================

pub struct ExfatFs {
    /// Диск общий с открытыми файлами: они читают по требованию.
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    boot: ExfatBoot,
    mounted: bool,
}

impl ExfatFs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        let mut bs = [0u8; 512];
        disk.read(0, &mut bs).map_err(|_| FsError::IoError)?;
        let boot = ExfatBoot::parse(&bs)?;

        Ok(Self {
            disk: Arc::new(Mutex::new(disk)),
            boot,
            mounted: false,
        })
    }
}

/// Открытый файл exFAT: читает кластеры по требованию.
pub struct ExfatFile {
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    boot: ExfatBoot,
    first_cluster: u32,
    size: u64,
    no_fat_chain: bool,
    position: u64,
    /// Кэш текущего кластера, чтобы не бегать по FAT на каждый байт.
    cached_cluster: u32,
    cached_index: u64,
}

impl ExfatFile {
    /// Кластер с номером `index` внутри файла.
    fn cluster_at(&mut self, index: u64) -> Result<u32, FsError> {
        if self.no_fat_chain {
            let c = self.first_cluster as u64 + index;
            if c >= self.boot.cluster_count as u64 + 2 {
                return Err(FsError::IoError);
            }
            return Ok(c as u32);
        }

        if self.cached_index > index || self.cached_cluster == 0 {
            self.cached_cluster = self.first_cluster;
            self.cached_index = 0;
        }
        while self.cached_index < index {
            match next_cluster(&self.disk, &self.boot, self.cached_cluster)? {
                Some(next) => {
                    self.cached_cluster = next;
                    self.cached_index += 1;
                }
                None => return Err(FsError::IoError),
            }
        }
        Ok(self.cached_cluster)
    }

    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), FsError> {
        let cluster_size = self.boot.cluster_size();
        if cluster_size == 0 {
            return Err(FsError::IoError);
        }
        let mut done = 0usize;
        while done < out.len() {
            let abs = offset + done as u64;
            let index = abs / cluster_size;
            let in_cluster = (abs % cluster_size) as usize;
            let cluster = self.cluster_at(index)?;

            let take = (out.len() - done).min(cluster_size as usize - in_cluster);
            let start = in_cluster % 512;
            let need = start + take;
            let sectors = (need + 511) / 512;

            let mut tmp = vec![0u8; sectors * 512];
            let lba = self.boot.cluster_lba(cluster) + (in_cluster / 512) as u64;
            self.disk
                .lock()
                .read(lba, &mut tmp)
                .map_err(|_| FsError::IoError)?;
            out[done..done + take].copy_from_slice(&tmp[start..start + take]);
            done += take;
        }
        Ok(())
    }
}

impl File for ExfatFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        if buf.is_empty() || self.position >= self.size {
            return Ok(0);
        }
        let n = buf.len().min((self.size - self.position) as usize);
        let offset = self.position;
        self.read_at(offset, &mut buf[..n])?;
        self.position += n as u64;
        Ok(n)
    }

    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::Current(o) => self.position as i64 + o,
            SeekFrom::End(o) => self.size as i64 + o,
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
        self.size as usize
    }
}

impl FileSystem for ExfatFs {
    fn name(&self) -> &str {
        "exfat"
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
        let entry = find_entry(&self.disk, &self.boot, path)?.ok_or(FsError::NotFound)?;
        if entry.is_dir {
            return Err(FsError::IsADirectory);
        }
        Ok(Box::new(ExfatFile {
            disk: self.disk.clone(),
            boot: self.boot,
            first_cluster: entry.first_cluster,
            size: entry.size,
            no_fat_chain: entry.no_fat_chain,
            position: 0,
            cached_cluster: 0,
            cached_index: 0,
        }))
    }

    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> {
        Err(FsError::ReadOnly)
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

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let cluster = resolve_dir(&self.disk, &self.boot, path)?;
        let entries = list_entries(&self.disk, &self.boot, cluster)?;
        Ok(entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                file_type: if e.is_dir { FileType::Directory } else { FileType::File },
                size: e.size as usize,
                permissions: ro_permissions(),
                uid: 0,
                gid: 0,
            })
            .collect())
    }

    fn exists(&self, path: &str) -> bool {
        match find_entry(&self.disk, &self.boot, path) {
            Ok(Some(_)) => true,
            // Корень существует всегда.
            Ok(None) => path.split('/').filter(|p| !p.is_empty()).count() == 0,
            Err(_) => false,
        }
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let parts = path.split('/').filter(|p| !p.is_empty()).count();
        if parts == 0 {
            return Ok(FileMetadata {
                name: String::from("/"),
                file_type: FileType::Directory,
                size: 0,
                permissions: ro_permissions(),
                uid: 0,
                gid: 0,
                created: 0,
                modified: 0,
            });
        }
        let entry = find_entry(&self.disk, &self.boot, path)?.ok_or(FsError::NotFound)?;
        Ok(FileMetadata {
            name: entry.name,
            file_type: if entry.is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
            size: entry.size as usize,
            permissions: ro_permissions(),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
        })
    }
}
