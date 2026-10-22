// fs/partition.rs — разбор таблиц разделов (MBR и GPT) + окно в диск.
//
// Зачем это нужно: FAT32/NTFS/EXT живут не на «голом» диске, а на разделе.
// Драйвер файловой системы должен видеть сектор 0 как начало СВОЕГО раздела.
// PartitionDisk делает именно это: транслирует номера секторов, добавляя
// смещение раздела, и не даёт выйти за его границы.

use super::disk::DiskIo;
use super::vfs::FsError;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::format;
use spin::Mutex;

/// Тип раздела (определяется по таблице разделов, а не по содержимому).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PartKind {
    /// Раздел с ФС (FAT/NTFS/EXT — конкретика определится при монтировании).
    Data,
    /// EFI System Partition (FAT32 по спецификации).
    Efi,
    /// Linux-раздел (ext2/3/4, XFS...).
    Linux,
    /// Неизвестный/служебный раздел.
    Unknown,
}

impl PartKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PartKind::Data => "data",
            PartKind::Efi => "efi",
            PartKind::Linux => "linux",
            PartKind::Unknown => "unknown",
        }
    }
}

/// Описание раздела.
#[derive(Debug, Clone)]
pub struct Partition {
    pub index: u8,
    pub start_lba: u64,
    pub sectors: u64,
    pub kind: PartKind,
    pub label: String,
}

/// Окно в родительский диск: сектор 0 окна = `offset` родительского диска.
pub struct PartitionDisk {
    inner: Arc<Mutex<Box<dyn DiskIo>>>,
    offset: u64,
    sectors: u64,
}

impl PartitionDisk {
    pub fn new(inner: Arc<Mutex<Box<dyn DiskIo>>>, offset: u64, sectors: u64) -> Self {
        Self { inner, offset, sectors }
    }

    pub fn sectors(&self) -> u64 { self.sectors }

    fn check(&self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        if buf.is_empty() || buf.len() % 512 != 0 { return Err(FsError::InvalidPath); }
        let count = (buf.len() / 512) as u64;
        if sector.checked_add(count).map_or(true, |end| end > self.sectors) {
            return Err(FsError::IoError);
        }
        Ok(())
    }
}

impl DiskIo for PartitionDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        self.check(sector, buf)?;
        self.inner.lock().read(self.offset + sector, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        self.check(sector, buf)?;
        self.inner.lock().write(self.offset + sector, buf)
    }
}

// ==========================================
// CRC32 (нужен для проверки GPT)
// ==========================================

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn u64_at(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off], buf[off + 1], buf[off + 2], buf[off + 3],
        buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7],
    ])
}

/// Сравнить GPT GUID в записи с эталоном.
fn guid_eq(buf: &[u8], off: usize, guid: &[u8; 16]) -> bool {
    &buf[off..off + 16] == guid
}

/// GPT-имена хранятся в UTF-16LE.
fn utf16_name(buf: &[u8], off: usize, max_chars: usize) -> String {
    let mut s = String::new();
    for i in 0..max_chars {
        let c = u16::from_le_bytes([buf[off + i * 2], buf[off + i * 2 + 1]]);
        if c == 0 { break; }
        s.push(char::from_u32(c as u32).unwrap_or('?'));
    }
    s
}

// ==========================================
// MBR
// ==========================================

/// MBR: 4 записи по 16 байт с offset 446, сигнатура 0x55AA в 510.
fn parse_mbr(sector0: &[u8]) -> Option<Vec<Partition>> {
    if sector0[510] != 0x55 || sector0[511] != 0xAA { return None; }

    let mut parts = Vec::new();
    for i in 0..4usize {
        let off = 446 + i * 16;
        let ptype = sector0[off + 4];
        let start = u32_at(sector0, off + 8) as u64;
        let sectors = u32_at(sector0, off + 12) as u64;
        if ptype == 0 || sectors == 0 { continue; }

        // 0xEE = protective MBR, значит настоящая таблица — GPT.
        if ptype == 0xEE { return None; }

        let kind = match ptype {
            0x0B | 0x0C | 0x0E | 0x06 | 0x01 | 0x04 => PartKind::Data, // FAT
            0x07 => PartKind::Data,                                     // NTFS/exFAT
            0x83 => PartKind::Linux,
            0xEF => PartKind::Efi,
            _ => PartKind::Unknown,
        };

        parts.push(Partition {
            index: (i + 1) as u8,
            start_lba: start,
            sectors,
            kind,
            label: String::new(),
        });
    }

    if parts.is_empty() { None } else { Some(parts) }
}

// ==========================================
// GPT
// ==========================================

const GPT_SIG: &[u8; 8] = b"EFI PART";

// Microsoft basic data (FAT/NTFS/exFAT)
const GUID_MS_BASIC_DATA: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44,
    0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];
// EFI System Partition
const GUID_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
    0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];
// Linux filesystem data
const GUID_LINUX_DATA: [u8; 16] = [
    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47,
    0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4,
];

/// Читает GPT-заголовок (LBA 1) и массив записей разделов.
fn parse_gpt(disk: &mut dyn DiskIo) -> Option<Vec<Partition>> {
    let mut hdr = [0u8; 512];
    disk.read(1, &mut hdr).ok()?;
    if &hdr[0..8] != GPT_SIG { return None; }

    let header_size = u32_at(&hdr, 12) as usize;
    if !(92..=512).contains(&header_size) { return None; }

    // CRC заголовка считается при обнулённом поле CRC (offset 16..20).
    let mut copy = [0u8; 512];
    copy[..header_size].copy_from_slice(&hdr[..header_size]);
    let stored_crc = u32_at(&hdr, 16);
    copy[16..20].fill(0);
    if crc32(&copy[..header_size]) != stored_crc { return None; }

    let entries_lba = u64_at(&hdr, 72);
    let num_entries = u32_at(&hdr, 80) as usize;
    let entry_size = u32_at(&hdr, 84) as usize;
    if entry_size < 128 || num_entries == 0 || num_entries > 1024 { return None; }

    // Читаем записи одним куском, чтобы не делать по сектору на запись.
    let total = num_entries * entry_size;
    let sectors = (total + 511) / 512;
    let mut buf = alloc::vec![0u8; sectors * 512];
    disk.read(entries_lba, &mut buf).ok()?;

    let mut parts = Vec::new();
    for i in 0..num_entries {
        let off = i * entry_size;
        // Нулевой type GUID = неиспользуемая запись.
        if buf[off..off + 16].iter().all(|&b| b == 0) { continue; }

        let first = u64_at(&buf, off + 32);
        let last = u64_at(&buf, off + 40);
        if last < first { continue; }

        let kind = if guid_eq(&buf, off, &GUID_EFI_SYSTEM) {
            PartKind::Efi
        } else if guid_eq(&buf, off, &GUID_LINUX_DATA) {
            PartKind::Linux
        } else if guid_eq(&buf, off, &GUID_MS_BASIC_DATA) {
            PartKind::Data
        } else {
            PartKind::Unknown
        };

        parts.push(Partition {
            index: (i + 1) as u8,
            start_lba: first,
            sectors: last - first + 1,
            kind,
            label: utf16_name(&buf, off + 56, 36),
        });
    }

    if parts.is_empty() { None } else { Some(parts) }
}

/// Разобрать таблицу разделов диска.
///
/// Возвращает пустой вектор, если таблицы нет (диск отформатирован целиком,
/// «superfloppy» FAT32) — тогда монтировать нужно сам диск.
pub fn read_partitions(disk: &mut dyn DiskIo) -> Vec<Partition> {
    let mut sector0 = [0u8; 512];
    if disk.read(0, &mut sector0).is_err() { return Vec::new(); }

    // Сначала GPT (protective MBR отличим по типу 0xEE), потом MBR.
    if sector0[446 + 4] == 0xEE {
        if let Some(parts) = parse_gpt(disk) { return parts; }
    }
    if let Some(parts) = parse_mbr(&sector0) { return parts; }
    Vec::new()
}

/// Определить тип ФС по первым секторам раздела.
pub fn detect_fs_type(disk: &mut dyn DiskIo) -> &'static str {
    let mut s = [0u8; 512];
    if disk.read(0, &mut s).is_err() { return "unknown"; }

    // NTFS: OEM ID "NTFS    " по смещению 3.
    if &s[3..11] == b"NTFS    " { return "ntfs"; }

    // exFAT: "EXFAT   " по смещению 3.
    if &s[3..11] == b"EXFAT   " { return "exfat"; }

    // FAT32: сигнатура 0x55AA + "FAT32" в поле типа ФС (offset 82).
    if s[510] == 0x55 && s[511] == 0xAA && &s[82..87] == b"FAT32" { return "fat32"; }

    // ext2/3/4: магия 0xEF53 в суперблоке (смещение 1024 + 56 = 1080,
    // то есть байт 56 в секторе 2).
    let mut sb = [0u8; 512];
    if disk.read(2, &mut sb).is_ok() && u16::from_le_bytes([sb[56], sb[57]]) == 0xEF53 {
        return "ext2";
    }

    "unknown"
}

/// Имя раздела: `sda` + `1` → `sda1`.
pub fn part_name(disk_name: &str, index: u8) -> String {
    format!("{}{}", disk_name, index)
}

