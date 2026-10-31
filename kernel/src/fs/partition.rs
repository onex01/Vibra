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
///
/// ЕДИНИЦА ИЗМЕРЕНИЯ: `offset` и `sectors` всегда в 512-байтных секторах,
/// независимо от размера логического сектора устройства. Раньше здесь
/// смешивались 512-байтные сектора таблицы разделов и `sector_size()`
/// (4096 на 4Kn), из-за чего окно раздела уезжало в 8 раз дальше и ФС
/// читалась как «unknown».
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
        let len_sectors = (buf.len() / 512) as u64;
        let end = sector.checked_add(len_sectors).ok_or(FsError::IoError)?;
        if end > self.sectors {
            return Err(FsError::IoError);
        }
        Ok(())
    }
}

impl DiskIo for PartitionDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        self.check(sector, buf)?;
        let target = self.offset.checked_add(sector).ok_or(FsError::IoError)?;
        self.inner.lock().read(target, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        self.check(sector, buf)?;
        let target = self.offset.checked_add(sector).ok_or(FsError::IoError)?;
        self.inner.lock().write(target, buf)
    }

    fn sector_size(&self) -> usize {
        self.inner.lock().sector_size()
    }

    fn is_read_only(&self) -> bool {
        self.inner.lock().is_read_only()
    }

    fn read_lba(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FsError> {
        // LBA — в логических секторах устройства, а `offset` — в 512-байтных.
        let ss = self.inner.lock().sector_size().max(512) as u64;
        let byte = self.offset
            .checked_mul(512).ok_or(FsError::IoError)?
            .checked_add(lba.checked_mul(ss).ok_or(FsError::IoError)?)
            .ok_or(FsError::IoError)?;
        if byte % 512 != 0 {
            return Err(FsError::InvalidPath);
        }
        self.inner.lock().read(byte / 512, buf)
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
    match buf.get(off..off + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

fn u64_at(buf: &[u8], off: usize) -> u64 {
    match buf.get(off..off + 8) {
        Some(s) => u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]),
        None => 0,
    }
}

/// Сравнить GPT GUID в записи с эталоном.
fn guid_eq(buf: &[u8], off: usize, guid: &[u8; 16]) -> bool {
    matches!(buf.get(off..off + 16), Some(s) if s == guid)
}

/// GPT-имена хранятся в UTF-16LE.
fn utf16_name(buf: &[u8], off: usize, max_chars: usize) -> String {
    let mut s = String::new();
    for i in 0..max_chars {
        let Some(raw) = buf.get(off + i * 2..off + i * 2 + 2) else {
            break;
        };
        let c = u16::from_le_bytes([raw[0], raw[1]]);
        if c == 0 { break; }
        s.push(char::from_u32(c as u32).unwrap_or('?'));
    }
    s
}

// ==========================================
// MBR
// ==========================================

/// MBR: 4 записи по 16 байт с offset 446, сигнатура 0x55AA в 510.
///
/// Поддерживаются extended-разделы (типы 0x05/0x0F/0x85): логические разделы
/// нумеруются с 5 и идут цепочкой EBR. Без этого два NTFS-раздела на MBR-
/// диске оставались «unknown», потому что в primary-записях лежал только
/// контейнер extended.
fn kind_from_mbr(ptype: u8) -> PartKind {
    match ptype {
        // FAT12/16/32 и скрытые варианты.
        0x01 | 0x04 | 0x06 | 0x0B | 0x0C | 0x0E | 0x11 | 0x14 | 0x16 | 0x1B | 0x1C
        | 0x1E => PartKind::Data,
        // NTFS/exFAT.
        0x07 | 0x17 | 0x27 => PartKind::Data,
        0x83 => PartKind::Linux,
        0xEF => PartKind::Efi,
        _ => PartKind::Unknown,
    }
}

fn read_mbr_sector(disk: &mut dyn DiskIo, lba: u64) -> Option<[u8; 512]> {
    let mut sector = [0u8; 512];
    disk.read(lba, &mut sector).ok()?;
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return None;
    }
    Some(sector)
}

fn mbr_entry(sector: &[u8], index: usize) -> (u8, u64, u64) {
    let off = 446 + index * 16;
    let ptype = sector[off + 4];
    let start = u32_at(sector, off + 8) as u64;
    let sectors = u32_at(sector, off + 12) as u64;
    (ptype, start, sectors)
}

fn parse_mbr(disk: &mut dyn DiskIo, sector0: &[u8]) -> Option<Vec<Partition>> {
    let mut parts = Vec::new();
    let mut extended_start: Option<u64> = None;

    for i in 0..4usize {
        let (ptype, start, sectors) = mbr_entry(sector0, i);
        if ptype == 0 || sectors == 0 {
            continue;
        }
        // 0xEE = protective MBR. В hybrid-образе (debug ISO) рядом с ним
        // лежат обычные записи appended-разделов, поэтому не выходим сразу,
        // а просто пропускаем защитную запись.
        if ptype == 0xEE {
            continue;
        }
        if ptype == 0x05 || ptype == 0x0F || ptype == 0x85 {
            extended_start = Some(start);
            continue;
        }

        parts.push(Partition {
            index: (i + 1) as u8,
            start_lba: start,
            sectors,
            kind: kind_from_mbr(ptype),
            label: String::new(),
        });
    }

    // Логические разделы внутри extended: EBR-цепочка.
    if let Some(extended) = extended_start {
        let mut ebr_lba = extended;
        let mut logical_index = 5u8;
        // Ограничение от зацикливания на повреждённой цепочке.
        for _ in 0..128 {
            let Some(ebr) = read_mbr_sector(disk, ebr_lba) else {
                break;
            };

            // Запись 0 — сам логический раздел (start относительно EBR).
            let (ptype, rel_start, sectors) = mbr_entry(&ebr, 0);
            if ptype != 0 && sectors != 0 {
                parts.push(Partition {
                    index: logical_index,
                    start_lba: ebr_lba.checked_add(rel_start)?,
                    sectors,
                    kind: kind_from_mbr(ptype),
                    label: String::new(),
                });
                logical_index = logical_index.saturating_add(1);
            }

            // Запись 1 — ссылка на следующий EBR (start относительно extended).
            let (next_type, next_rel, next_sectors) = mbr_entry(&ebr, 1);
            if next_type == 0 || next_sectors == 0 {
                break;
            }
            let next = extended.checked_add(next_rel)?;
            if next == ebr_lba {
                break;
            }
            ebr_lba = next;
        }
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
    let ss = disk.sector_size().max(512);
    let mut hdr = [0u8; 512];
    disk.read_lba(1, &mut hdr).ok()?;
    if hdr.len() < 92 || &hdr[0..8] != GPT_SIG { return None; }

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
    if !(128..=4096).contains(&entry_size) || num_entries == 0 || num_entries > 1024 {
        return None;
    }
    if entry_size > ss { return None; }

    let mut part = [0u8; 4096];
    let mut parts = Vec::new();

    // Записи GPT лежат подряд в LBA-секторах устройства. На 4Kn одна
    // запись может занимать часть 4096-байтного сектора, поэтому читаем
    // по Logical Block Address, а не байтовым смещением/512.
    for i in 0..num_entries {
        let byte_offset = i.checked_mul(entry_size)?;
        let lba = entries_lba.checked_add((byte_offset / ss) as u64)?;
        let in_sector = byte_offset % ss;
        if in_sector + entry_size > ss {
            // Запись на границе секторов — читаем два сектора подряд.
            let mut two = alloc::vec![0u8; ss * 2];
            disk.read_lba(lba, &mut two[..ss]).ok()?;
            disk.read_lba(lba + 1, &mut two[ss..]).ok()?;
            part[..entry_size]
                .copy_from_slice(&two[in_sector..in_sector + entry_size]);
        } else {
            let mut sector = alloc::vec![0u8; ss];
            disk.read_lba(lba, &mut sector).ok()?;
            part[..entry_size]
                .copy_from_slice(&sector[in_sector..in_sector + entry_size]);
        }
        let entry = &part[..entry_size];

        // Нулевой type GUID = неиспользуемая запись.
        if entry[..16].iter().all(|&b| b == 0) { continue; }

        let first = u64_at(entry, 32);
        let last = u64_at(entry, 40);
        if last < first { continue; }

        // GPT хранит LBA в логических секторах устройства. Внутри ядра все
        // разделы живут в 512-байтных единицах, поэтому на 4Kn умножаем на 8.
        let ratio = (ss / 512).max(1) as u64;
        let start_lba = match first.checked_mul(ratio) {
            Some(value) => value,
            None => continue,
        };
        let sectors = match (last - first + 1).checked_mul(ratio) {
            Some(value) => value,
            None => continue,
        };

        let kind = if guid_eq(entry, 0, &GUID_EFI_SYSTEM) {
            PartKind::Efi
        } else if guid_eq(entry, 0, &GUID_LINUX_DATA) {
            PartKind::Linux
        } else if guid_eq(entry, 0, &GUID_MS_BASIC_DATA) {
            PartKind::Data
        } else {
            PartKind::Unknown
        };

        parts.push(Partition {
            index: (i + 1) as u8,
            start_lba,
            sectors,
            kind,
            label: utf16_name(entry, 56, 36),
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

    // Protective MBR может стоять в любой из четырёх записей (спецификация
    // требует первую, но некоторые загрузчики/утилиты кладут её иначе).
    let has_protective = (0..4).any(|i| mbr_entry(&sector0, i).0 == 0xEE);

    // Сначала GPT: пробуем всегда, если есть protective MBR или валидный
    // заголовок "EFI PART" в LBA 1.
    if has_protective {
        if let Some(parts) = parse_gpt(disk) {
            return parts;
        }
    }
    if let Some(parts) = parse_mbr(disk, &sector0) {
        return parts;
    }
    // GPT без protective MBR (редко, но бывает у обрезанных образов).
    if let Some(parts) = parse_gpt(disk) {
        return parts;
    }
    Vec::new()
}

/// Определить тип ФС по первым секторам раздела.
pub fn detect_fs_type(disk: &mut dyn DiskIo) -> &'static str {
    // Читаем 2 КиБ: NTFS/FAT/exFAT signature находятся в первом секторе,
    // ext2 superblock — по байтовому смещению 1024. На 4Kn-диске это
    // внутри первого 4096-байтного блока, поэтому нельзя просто читать
    // "сектор 2" как 512-байтный.
    let mut s = [0u8; 2048];
    if disk.read(0, &mut s).is_err() { return "unknown"; }

    // NTFS: OEM ID "NTFS    " по смещению 3.
    if &s[3..11] == b"NTFS    " { return "ntfs"; }

    // exFAT: "EXFAT   " по смещению 3.
    if &s[3..11] == b"EXFAT   " { return "exfat"; }

    // FAT32: сигнатура 0x55AA + "FAT32" в поле типа ФС (offset 82).
    if s[510] == 0x55 && s[511] == 0xAA && &s[82..87] == b"FAT32" { return "fat32"; }

    // ext2/3/4: магия 0xEF53 в суперблоке (смещение 1024 + 56).
    if u16::from_le_bytes([s[1024 + 56], s[1024 + 57]]) == 0xEF53 {
        // Различаем поколения по feature-флагам суперблока.
        let compat = u32_at(&s, 1024 + 92);
        let incompat = u32_at(&s, 1024 + 96);
        let ro_compat = u32_at(&s, 1024 + 100);
        let is_ext4 = incompat & (0x40 | 0x80 | 0x200) != 0 // EXTENTS|64BIT|FLEX_BG
            || ro_compat & 0x200 != 0; // METADATA_CSUM
        if is_ext4 {
            return "ext4";
        }
        if compat & 0x4 != 0 {
            // HAS_JOURNAL
            return "ext3";
        }
        return "ext2";
    }

    "unknown"
}

/// Имя раздела: `sda` + `1` → `sda1`, `sr0` + `2` → `sr0p2`.
///
/// Как в Linux: если имя устройства заканчивается цифрой (sr0, nvme0n1),
/// добавляется `p`, иначе `sr02` читалось бы как отдельное устройство.
pub fn part_name(disk_name: &str, index: u8) -> String {
    let ends_with_digit = disk_name
        .as_bytes()
        .last()
        .map(|byte| byte.is_ascii_digit())
        .unwrap_or(false);
    if ends_with_digit {
        format!("{}p{}", disk_name, index)
    } else {
        format!("{}{}", disk_name, index)
    }
}

