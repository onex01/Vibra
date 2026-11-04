// fs/iso9660.rs — ISO9660 (CD/DVD), только чтение.
//
// Зачем: `srX` до этого был просто блочным устройством, и диск, с которого
// ядро грузится, нельзя было даже посмотреть. Драйвер закрывает базовый
// случай — data CD/DVD, собранный genisoimage/xorriso:
//
//   * Primary Volume Descriptor в LBA 16 (2048 байт), «CD001»;
//   * Joliet (Supplementary VD с escape-последовательностями %/@, %/C, %/E):
//     имена в UCS-2 BE, поэтому длинные имена и нижний регистр видны как есть;
//   * Rock Ridge `NM` в System Use — если Joliet нет, длинные имена всё равно
//     читаются на Linux-образах;
//   * каталоги (directory records), подкаталоги, файлы.
//
// extents в ISO9660 непрерывны по определению, поэтому файл читается
// по требованию без цепочек: смещение = LBA extent'а + позиция.
//
// Не реализовано: multi-extent (флаг 0x80), continuation area (CE),
// interleaved-файлы, El Torito. Такие записи пропускаются.

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

/// Первый сектор тома, где лежат volume descriptors (в 2048-байтных блоках).
const VD_LBA: u64 = 16;
/// Классический размер блока ISO9660; PVD может объявить другой.
const DEFAULT_BLOCK: u32 = 2048;
/// Предел на чтение одного каталога.
const MAX_DIR_BYTES: usize = 64 * 1024 * 1024;

fn rd_le_u32(s: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([s[off], s[off + 1], s[off + 2], s[off + 3]])
}

fn rd_le_u16(s: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([s[off], s[off + 1]])
}

/// Запись каталога ISO9660 (directory record).
#[derive(Clone)]
struct Record {
    name: String,
    lba: u32,
    size: u32,
    is_dir: bool,
}

/// Прочитать `out.len()` байт по абсолютному байтовому смещению.
///
/// `DiskIo::read` работает 512-байтными единицами, поэтому смещение и размер
/// приводятся к ним, а лишнее отбрасывается.
fn read_abs(
    disk: &Mutex<Box<dyn DiskIo>>,
    byte_offset: u64,
    out: &mut [u8],
) -> Result<(), FsError> {
    if out.is_empty() {
        return Ok(());
    }
    let sector = byte_offset / 512;
    let start = (byte_offset % 512) as usize;
    let need = start + out.len();
    let sectors = (need + 511) / 512;
    let mut tmp = vec![0u8; sectors * 512];
    disk.lock()
        .read(sector, &mut tmp)
        .map_err(|_| FsError::IoError)?;
    out.copy_from_slice(&tmp[start..start + out.len()]);
    Ok(())
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

/// Имя из ISO-записи: убрать `;1` и завершающую точку.
fn iso_name(raw: &[u8]) -> String {
    let mut end = raw.len();
    for (i, &b) in raw.iter().enumerate() {
        if b == b';' {
            end = i;
            break;
        }
    }
    let mut text = String::from_utf8_lossy(&raw[..end]).into_owned();
    while text.ends_with('.') {
        text.pop();
    }
    text
}

/// Имя из Joliet-записи: UCS-2 big-endian.
fn joliet_name(raw: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut i = 0;
    while i + 1 < raw.len() {
        units.push(u16::from_be_bytes([raw[i], raw[i + 1]]));
        i += 2;
    }
    // Отсечь `;1` вместе с версией.
    if let Some(pos) = units.iter().position(|&u| u == u16::from(b';')) {
        units.truncate(pos);
    }
    let mut out = String::new();
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() {
            let lo = units[i + 1];
            if (0xDC00..0xE000).contains(&lo) {
                let cp = 0x10000 + (((u - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
                out.push(char::from_u32(cp).unwrap_or('?'));
                i += 2;
                continue;
            }
        }
        out.push(char::from_u32(u as u32).unwrap_or('?'));
        i += 1;
    }
    while out.ends_with('.') {
        out.pop();
    }
    out
}

/// Имя из Rock Ridge `NM` в System Use-области (если она есть).
fn rock_ridge_name(sys: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut found = false;
    let mut i = 0usize;
    while i + 4 <= sys.len() {
        let len = sys[i + 2] as usize;
        if len < 4 || i + len > sys.len() {
            break;
        }
        let entry = &sys[i..i + len];
        if &entry[0..2] == b"NM" && entry.len() > 4 {
            // bit0 = 1 означает «продолжение в следующей записи NM».
            let flags = entry[4];
            if flags & 0x01 == 0 {
                let part = &entry[5..];
                if let Ok(text) = core::str::from_utf8(part) {
                    out.push_str(text);
                }
                found = true;
            }
        }
        i += len;
    }
    if found && !out.is_empty() {
        Some(out)
    } else {
        None
    }
}

/// Разобрать одну directory record.
fn parse_record(rec: &[u8], joliet: bool) -> Option<Record> {
    if rec.len() < 34 {
        return None;
    }
    let len = rec[0] as usize;
    if len < 34 || len > rec.len() {
        return None;
    }
    let lba = rd_le_u32(rec, 2);
    let size = rd_le_u32(rec, 10);
    let flags = rec[25];
    let name_len = rec[32] as usize;
    if 33 + name_len > len {
        return None;
    }
    let raw_name = &rec[33..33 + name_len];

    // 0x00 — «.», 0x01 — «..»: в листинге не нужны.
    if name_len == 1 && raw_name[0] <= 1 {
        return None;
    }

    // System Use начинается после имени, выровненного до чётного.
    let sys_start = 33 + name_len + (name_len % 2 == 0) as usize;
    let sys: &[u8] = if sys_start < len { &rec[sys_start..len] } else { &[] };

    let name = if joliet {
        joliet_name(raw_name)
    } else {
        rock_ridge_name(sys).unwrap_or_else(|| iso_name(raw_name))
    };

    if name.is_empty() {
        return None;
    }

    Some(Record {
        name,
        lba,
        size,
        is_dir: flags & 0x02 != 0,
    })
}

/// Метка тома ISO9660 (Volume Identifier из PVD, 32 d-символа).
///
/// Нужна для `disks` и `mount ... LABEL=`: у CD/DVD нет BPB, и без этого
/// единственное имя тома — `sr0`.
pub fn volume_label(disk: &mut dyn DiskIo) -> Option<String> {
    let mut pvd = [0u8; 2048];
    disk.read(VD_LBA * (DEFAULT_BLOCK as u64 / 512), &mut pvd).ok()?;
    if &pvd[1..6] != b"CD001" || pvd[0] != 1 {
        return None;
    }
    let raw = &pvd[40..72];
    let end = raw.iter().position(|&b| b == b' ').unwrap_or(raw.len());
    let text = core::str::from_utf8(&raw[..end]).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(String::from(text))
    }
}

// ==========================================
// Файловая система
// ==========================================

pub struct IsoFs {
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    block_size: u32,
    root_lba: u32,
    root_size: u32,
    joliet: bool,
    mounted: bool,
}

impl IsoFs {
    pub fn new(disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        let disk: Arc<Mutex<Box<dyn DiskIo>>> = Arc::new(Mutex::new(disk));
        let mut primary: Option<(u32, u32, u32)> = None; // block, root_lba, root_size
        let mut supplementary: Option<(u32, u32)> = None; // root_lba, root_size

        // Volume descriptors: блок 16, затем по одному на блок, до терминатора
        // 255. Ограничение — 32 записи, чтобы не зациклиться на битом томе.
        let mut vd = vec![0u8; DEFAULT_BLOCK as usize];
        for i in 0..32u64 {
            let off = (VD_LBA + i) * DEFAULT_BLOCK as u64;
            if read_abs(&disk, off, &mut vd).is_err() {
                break;
            }
            if &vd[1..6] != b"CD001" {
                break;
            }
            match vd[0] {
                1 => {
                    let block = rd_le_u16(&vd, 128) as u32;
                    let block = if block == 0 { DEFAULT_BLOCK } else { block };
                    let root_lba = rd_le_u32(&vd, 156 + 2);
                    let root_size = rd_le_u32(&vd, 156 + 10);
                    primary = Some((block, root_lba, root_size));
                }
                2 => {
                    // Joliet определяем по escape-последовательности.
                    let esc = &vd[88..91];
                    if esc == b"%/@" || esc == b"%/C" || esc == b"%/E" {
                        let root_lba = rd_le_u32(&vd, 156 + 2);
                        let root_size = rd_le_u32(&vd, 156 + 10);
                        supplementary = Some((root_lba, root_size));
                    }
                }
                // 0 — boot record, 3 — partition, 255 — терминатор.
                255 => break,
                _ => {}
            }
        }

        let (block_size, root_lba, root_size) = primary.ok_or(FsError::InvalidPath)?;
        // Joliet-каталог использует ту же геометрию, что и primary.
        let (root_lba, root_size, joliet) = match supplementary {
            Some((lba, size)) => (lba, size, true),
            None => (root_lba, root_size, false),
        };
        if root_lba == 0 || root_size == 0 {
            return Err(FsError::InvalidPath);
        }

        Ok(Self {
            disk,
            block_size,
            root_lba,
            root_size,
            joliet,
            mounted: false,
        })
    }

    /// Список записей каталога.
    fn list(&self, lba: u32, size: u32) -> Result<Vec<Record>, FsError> {
        let size = (size as usize).min(MAX_DIR_BYTES);
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut data = vec![0u8; size];
        let off = lba as u64 * self.block_size as u64;
        read_abs(&self.disk, off, &mut data)?;

        let mut out = Vec::new();
        let mut i = 0usize;
        while i < data.len() {
            let len = data[i] as usize;
            // 0 — конец каталога (дальше только нули до конца блока).
            if len == 0 {
                // Перейти к следующему блоку: записи не пересекают границу.
                let next = ((i / self.block_size as usize) + 1) * self.block_size as usize;
                if next <= i {
                    break;
                }
                i = next;
                continue;
            }
            if i + len > data.len() {
                break;
            }
            if let Some(rec) = parse_record(&data[i..i + len], self.joliet) {
                out.push(rec);
            }
            i += len;
        }
        Ok(out)
    }

    fn find(&self, path: &str) -> Result<Option<Record>, FsError> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Ok(None);
        }

        let mut lba = self.root_lba;
        let mut size = self.root_size;
        for (i, part) in parts.iter().enumerate() {
            let records = self.list(lba, size)?;
            let Some(rec) = records.into_iter().find(|r| r.name == *part) else {
                return Ok(None);
            };
            if i + 1 == parts.len() {
                return Ok(Some(rec));
            }
            if !rec.is_dir {
                return Ok(None);
            }
            lba = rec.lba;
            size = rec.size;
        }
        Ok(None)
    }
}

/// Открытый файл ISO9660: extent непрерывен, читаем по требованию.
pub struct IsoFile {
    disk: Arc<Mutex<Box<dyn DiskIo>>>,
    base: u64,
    size: u64,
    position: u64,
}

impl File for IsoFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        if buf.is_empty() || self.position >= self.size {
            return Ok(0);
        }
        let n = buf.len().min((self.size - self.position) as usize);
        read_abs(&self.disk, self.base + self.position, &mut buf[..n])?;
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

impl FileSystem for IsoFs {
    fn name(&self) -> &str {
        "iso9660"
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
        let rec = self.find(path)?.ok_or(FsError::NotFound)?;
        if rec.is_dir {
            return Err(FsError::IsADirectory);
        }
        Ok(Box::new(IsoFile {
            disk: self.disk.clone(),
            base: rec.lba as u64 * self.block_size as u64,
            size: rec.size as u64,
            position: 0,
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
        let parts = path.split('/').filter(|p| !p.is_empty()).count();
        let (lba, size) = if parts == 0 {
            (self.root_lba, self.root_size)
        } else {
            match self.find(path)? {
                Some(r) if r.is_dir => (r.lba, r.size),
                Some(_) => return Err(FsError::NotADirectory),
                None => return Err(FsError::NotFound),
            }
        };
        Ok(self
            .list(lba, size)?
            .into_iter()
            .map(|r| DirEntry {
                name: r.name,
                file_type: if r.is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
                size: r.size as usize,
                permissions: ro_permissions(),
                uid: 0,
                gid: 0,
            })
            .collect())
    }

    fn exists(&self, path: &str) -> bool {
        match self.find(path) {
            Ok(Some(_)) => true,
            Ok(None) => path.split('/').filter(|p| !p.is_empty()).count() == 0,
            Err(_) => false,
        }
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        if path.split('/').filter(|p| !p.is_empty()).count() == 0 {
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
        let rec = self.find(path)?.ok_or(FsError::NotFound)?;
        Ok(FileMetadata {
            name: rec.name,
            file_type: if rec.is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
            size: rec.size as usize,
            permissions: ro_permissions(),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
        })
    }
}
