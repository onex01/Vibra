// fs/ntfs.rs — NTFS только для чтения.
//
// Реализовано: загрузочный сектор, MFT, атрибуты (resident и non-resident),
// data runs, каталоги через $INDEX_ROOT + $INDEX_ALLOCATION.
//
// Ограничения (сознательные, чтобы драйвер оставался компактным):
//   * только чтение — запись/создание/удаление возвращают FsError::ReadOnly;
//   * сжатые (compressed) и зашифрованные (EFS) файлы не поддерживаются;
//   * имена берутся в UTF-16 → UTF-8 (символы вне BMP превращаются в '?');
//   * файл читается в память целиком (как и в FAT32-драйвере).

use super::disk::DiskIo;
use super::vfs::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

const ATTR_DATA: u32 = 0x80;
const ATTR_INDEX_ROOT: u32 = 0x90;
const ATTR_INDEX_ALLOCATION: u32 = 0xA0;

/// Максимальный размер файла, который готовы прочитать целиком.
const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

/// Загрузочный сектор NTFS (нужные поля).
struct NtfsBoot {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    total_sectors: u64,
    /// LCN (номер кластера) начала таблицы $MFT. Именно кластер, а не LBA:
    /// при кластере 4096 байт LCN 4 — это LBA 32.
    mft_lcn: u64,
    record_size: u32,
}

impl NtfsBoot {
    fn parse(s: &[u8]) -> Result<Self, FsError> {
        if s.len() < 512 { return Err(FsError::InvalidPath); }
        if &s[3..11] != b"NTFS    " { return Err(FsError::InvalidPath); }

        let bytes_per_sector = u16::from_le_bytes([s[11], s[12]]);
        let sectors_per_cluster = s[13];
        if bytes_per_sector < 256 || !bytes_per_sector.is_power_of_two() { return Err(FsError::InvalidPath); }
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() { return Err(FsError::InvalidPath); }

        let total_sectors = u64::from_le_bytes([
            s[40], s[41], s[42], s[43], s[44], s[45], s[46], s[47],
        ]);
        let mft_lcn = u64::from_le_bytes([
            s[48], s[49], s[50], s[51], s[52], s[53], s[54], s[55],
        ]);
        if total_sectors == 0 || mft_lcn == 0 { return Err(FsError::InvalidPath); }

        // Кластеров на запись MFT — знаковое число: если отрицательное,
        // размер записи = 2^(-n) байт.
        let cpr = s[64] as i8;
        let record_size = if cpr > 0 {
            cpr as u32 * bytes_per_sector as u32 * sectors_per_cluster as u32
        } else if cpr < 0 {
            1u32 << (-(cpr as i32) as u32)
        } else {
            return Err(FsError::InvalidPath);
        };
        if record_size < 512 || record_size > 65536 { return Err(FsError::InvalidPath); }

        Ok(Self { bytes_per_sector, sectors_per_cluster, total_sectors, mft_lcn, record_size })
    }

    fn cluster_size(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }
}


// ==========================================
// Вспомогательные разборщики
// ==========================================

fn rd_u16(b: &[u8], o: usize) -> u16 {
    match b.get(o..o + 2) {
        Some(s) => u16::from_le_bytes([s[0], s[1]]),
        None => 0,
    }
}
fn rd_u32(b: &[u8], o: usize) -> u32 {
    match b.get(o..o + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}
fn rd_u64(b: &[u8], o: usize) -> u64 {
    match b.get(o..o + 8) {
        Some(s) => u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]),
        None => 0,
    }
}

/// UTF-16LE → String (символы вне BMP → '?').
fn utf16_to_string(b: &[u8], count: usize) -> String {
    let mut s = String::new();
    // Не читаем за границей среза, даже если count из заголовка больше.
    let count = count.min(b.len() / 2);
    let mut i = 0;
    while i < count {
        let u = rd_u16(b, i * 2);
        i += 1;
        if u == 0 { break; }
        if (0xD800..0xDC00).contains(&u) && i < count {
            let lo = rd_u16(b, i * 2);
            if (0xDC00..0xE000).contains(&lo) {
                i += 1;
                let cp = 0x10000 + (((u - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
                s.push(char::from_u32(cp).unwrap_or('?'));
                continue;
            }
        }
        s.push(char::from_u32(u as u32).unwrap_or('?'));
    }
    s
}

/// Один фрагмент data run: логический кластер → физический кластер.
#[derive(Clone, Copy)]
struct Run {
    vcn: u64,
    lcn: u64,
    count: u64,
}

/// Разобрать data runs (только несжатые).
///
/// Формат: байт-заголовок (младшие 4 бита — размер поля длины, старшие 4 —
/// размер поля смещения), затем длина (беззнаковая) и смещение (знаковое,
/// относительно предыдущего LCN). Заголовок 0 — конец списка.
fn parse_runs(b: &[u8]) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    let mut vcn = 0u64;
    let mut lcn = 0i64;

    while i < b.len() {
        let header = b[i];
        if header == 0 { break; }
        i += 1;
        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || len_size > 8 || off_size > 8 { break; }
        if i + len_size + off_size > b.len() { break; }

        let mut count = 0u64;
        for k in 0..len_size {
            count |= (b[i + k] as u64) << (8 * k);
        }
        i += len_size;

        let mut delta = 0i64;
        for k in 0..off_size {
            delta |= (b[i + k] as i64) << (8 * k);
        }
        // Знаковое расширение.
        if off_size > 0 && off_size < 8 {
            let sign_bit = 1i64 << (8 * off_size - 1);
            if delta & sign_bit != 0 {
                delta |= !((1i64 << (8 * off_size)) - 1);
            }
        }
        i += off_size;

        lcn += delta;
        if count == 0 { continue; }
        runs.push(Run { vcn, lcn: lcn as u64, count });
        vcn += count;
    }

    runs
}

/// Найти run, содержащий кластер `vcn`.
fn run_for(runs: &[Run], vcn: u64) -> Option<&Run> {
    runs.iter().find(|r| vcn >= r.vcn && vcn < r.vcn + r.count)
}

/// Обойти атрибуты записи MFT. Возвращает (смещение, тип) для каждого атрибута.
fn attr_offsets(rec: &[u8]) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    let first = rd_u16(rec, 20) as usize;
    let used = rd_u32(rec, 24) as usize;
    let limit = used.min(rec.len());

    let mut off = first;
    while off + 8 <= limit {
        let atype = rd_u32(rec, off);
        let alen = rd_u32(rec, off + 4) as usize;
        if atype == 0xFFFF_FFFF || alen < 24 || off + alen > limit { break; }
        out.push((off, atype));
        off += alen;
    }
    out
}

/// Полезная нагрузка resident-атрибута.
fn attr_resident_value<'a>(rec: &'a [u8], off: usize) -> Option<&'a [u8]> {
    if off.checked_add(24)? > rec.len() { return None; }
    if rec[off + 8] != 0 { return None; }
    let vlen = rd_u32(rec, off + 16) as usize;
    let voff = rd_u16(rec, off + 20) as usize;
    let start = off + voff;
    if start + vlen > rec.len() { return None; }
    Some(&rec[start..start + vlen])
}


/// Применить update sequence array (USA) — в NTFS последние два байта каждого
/// сектора записи заменены номером последовательности, а оригинал лежит в USA.
fn apply_fixup(rec: &mut [u8], bytes_per_sector: u16) {
    let usa_off = rd_u16(rec, 4) as usize;
    let usa_count = rd_u16(rec, 6) as usize;
    let bps = bytes_per_sector as usize;
    if usa_off == 0 || usa_count == 0 || bps == 0 { return; }
    if usa_off + usa_count * 2 > rec.len() { return; }

    for i in 1..usa_count {
        let sector_end = i * bps;
        if sector_end > rec.len() { break; }
        let src = usa_off + i * 2;
        rec[sector_end - 2] = rec[src];
        rec[sector_end - 1] = rec[src + 1];
    }
}

/// Прочитать `buf.len()` байт по абсолютному смещению.
fn read_abs(disk: &mut dyn DiskIo, byte_offset: u64, buf: &mut [u8]) -> Result<(), FsError> {
    let sector = byte_offset / 512;
    let in_sector = (byte_offset % 512) as usize;
    if in_sector == 0 && buf.len() % 512 == 0 {
        return disk.read(sector, buf).map_err(|_| FsError::IoError);
    }

    let sectors = (in_sector + buf.len() + 511) / 512;
    let mut tmp = vec![0u8; sectors * 512];
    disk.read(sector, &mut tmp).map_err(|_| FsError::IoError)?;
    buf.copy_from_slice(&tmp[in_sector..in_sector + buf.len()]);
    Ok(())
}

/// Описание файла из $FILE_NAME.
struct NameInfo {
    name: String,
    is_dir: bool,
    size: u64,
    /// 0 = POSIX, 1 = Win32, 2 = DOS 8.3, 3 = Win32+DOS
    namespace: u8,
}

/// Разобрать ключ индексной записи ($FILE_NAME без заголовка атрибута).
fn parse_name_key(key: &[u8]) -> Option<NameInfo> {
    if key.len() < 66 { return None; }
    let flags = rd_u32(key, 56);
    let name_len = key[64] as usize;
    let namespace = key[65];
    if name_len == 0 || 66 + name_len * 2 > key.len() { return None; }

    Some(NameInfo {
        name: utf16_to_string(&key[66..], name_len),
        is_dir: flags & 0x1000_0000 != 0,
        size: rd_u64(key, 48),
        namespace,
    })
}

// ==========================================
// Файловая система
// ==========================================

pub struct NtfsFs {
    disk: Mutex<Box<dyn DiskIo>>,
    boot: NtfsBoot,
    /// Размещение $MFT ($DATA записи 0).
    mft_runs: Vec<Run>,
    /// Реальный размер $MFT в байтах.
    mft_size: u64,
    mounted: bool,
}

impl NtfsFs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        let mut bs = [0u8; 512];
        disk.read(0, &mut bs).map_err(|_| FsError::IoError)?;
        let boot = NtfsBoot::parse(&bs)?;

        // Запись 0 ($MFT) лежит по смещению `mft_lcn` кластеров от начала тома.
        let rec0_off = boot.mft_lcn * boot.cluster_size() as u64;
        if rec0_off + boot.record_size as u64 > boot.total_sectors * 512 {
            return Err(FsError::InvalidPath);
        }
        let mut rec0 = vec![0u8; boot.record_size as usize];
        read_abs(disk.as_mut(), rec0_off, &mut rec0)?;
        apply_fixup(&mut rec0, boot.bytes_per_sector);

        if &rec0[0..4] != b"FILE" { return Err(FsError::InvalidPath); }

        let mut mft_runs = Vec::new();
        let mut mft_size = 0u64;

        for (off, atype) in attr_offsets(&rec0) {
            if atype != ATTR_DATA { continue; }
            // Безымянный $DATA — сам MFT.
            if rec0[off + 9] != 0 { continue; }
            if rec0[off + 8] == 0 { continue; } // resident быть не может

            let alen = rd_u32(&rec0, off + 4) as usize;
            let run_off = rd_u16(&rec0, off + 32) as usize;
            mft_size = rd_u64(&rec0, off + 48);
            if off + run_off < rec0.len() && off + alen <= rec0.len() {
                mft_runs = parse_runs(&rec0[off + run_off..off + alen]);
            }
            break;
        }

        if mft_runs.is_empty() { return Err(FsError::InvalidPath); }

        Ok(Self {
            disk: Mutex::new(disk),
            boot,
            mft_runs,
            mft_size,
            mounted: false,
        })
    }

    fn cluster_size(&self) -> u64 { self.boot.cluster_size() as u64 }

    /// Прочитать `out.len()` байт из цепочки runs по смещению `offset`.
    fn read_runs(&self, runs: &[Run], offset: u64, out: &mut [u8]) -> Result<(), FsError> {
        let cs = self.cluster_size();
        let mut done = 0usize;
        let mut disk = self.disk.lock();

        while done < out.len() {
            let abs = offset + done as u64;
            let vcn = abs / cs;
            let in_cluster = (abs % cs) as usize;

            let run = run_for(runs, vcn).ok_or(FsError::IoError)?;
            let lcn = run.lcn + (vcn - run.vcn);

            let take = (out.len() - done).min(cs as usize - in_cluster);
            let sectors_per_cluster = (cs / 512) as u64;
            let sector = lcn * sectors_per_cluster + (in_cluster / 512) as u64;
            let start = in_cluster % 512;

            let need = start + take;
            let nsectors = (need + 511) / 512;
            let mut tmp = vec![0u8; nsectors * 512];
            disk.read(sector, &mut tmp).map_err(|_| FsError::IoError)?;
            out[done..done + take].copy_from_slice(&tmp[start..start + take]);

            done += take;
        }
        Ok(())
    }

    /// Прочитать запись MFT по номеру (с применением fixup).
    fn read_record(&self, num: u64) -> Result<Vec<u8>, FsError> {
        if self.mft_size > 0 && num * self.boot.record_size as u64 >= self.mft_size {
            return Err(FsError::NotFound);
        }

        let mut rec = vec![0u8; self.boot.record_size as usize];
        self.read_runs(&self.mft_runs, num * self.boot.record_size as u64, &mut rec)?;

        if &rec[0..4] != b"FILE" { return Err(FsError::IoError); }
        apply_fixup(&mut rec, self.boot.bytes_per_sector);

        // Флаг 0x1 = запись используется.
        if rd_u16(&rec, 22) & 0x1 == 0 { return Err(FsError::NotFound); }
        Ok(rec)
    }


    /// Содержимое безымянного $DATA (файл целиком).
    fn file_data(&self, rec: &[u8]) -> Result<Vec<u8>, FsError> {
        for (off, atype) in attr_offsets(rec) {
            if atype != ATTR_DATA || rec[off + 9] != 0 { continue; }

            if rec[off + 8] == 0 {
                // Resident — данные прямо в записи.
                let v = attr_resident_value(rec, off).ok_or(FsError::IoError)?;
                return Ok(v.to_vec());
            }

            // Non-resident: runs + real_size.
            let alen = rd_u32(rec, off + 4) as usize;
            let run_off = rd_u16(rec, off + 32) as usize;
            if off + run_off > rec.len() || off + alen > rec.len() { return Err(FsError::IoError); }

            let real_size = rd_u64(rec, off + 48) as usize;
            if real_size > MAX_FILE_SIZE { return Err(FsError::NotSupported); }

            let runs = parse_runs(&rec[off + run_off..off + alen]);
            if runs.is_empty() { return Ok(Vec::new()); }

            let mut data = vec![0u8; real_size];
            if real_size > 0 {
                self.read_runs(&runs, 0, &mut data)?;
            }
            return Ok(data);
        }
        Err(FsError::NotFound)
    }


    /// Размер безымянного $DATA без чтения содержимого.
    fn data_size(&self, rec: &[u8]) -> usize {
        for (off, atype) in attr_offsets(rec) {
            if atype != ATTR_DATA || rec[off + 9] != 0 { continue; }
            if rec[off + 8] == 0 {
                // Resident: длина значения в заголовке атрибута.
                return rd_u32(rec, off + 16) as usize;
            }
            return rd_u64(rec, off + 48) as usize;
        }
        0
    }

    /// Список записей каталога: (имя, MFT-номер, это_каталог, размер).
    fn list_dir(&self, rec: &[u8]) -> Result<Vec<(String, u64, bool, u64)>, FsError> {
        let mut out: Vec<(String, u64, bool, u64)> = Vec::new();
        let attrs = attr_offsets(rec);
        let mut block_size = 4096usize;

        // --- $INDEX_ROOT ---
        for &(off, atype) in &attrs {
            if atype != ATTR_INDEX_ROOT { continue; }
            let Some(val) = attr_resident_value(rec, off) else { continue };
            if val.len() < 32 { continue; }

            block_size = rd_u32(val, 8) as usize;
            let entries_off = rd_u32(val, 16) as usize;
            let total = rd_u32(val, 20) as usize;
            let start = 16 + entries_off;
            let end = (16 + total).min(val.len());
            self.collect_entries(val, start, end, &mut out);
            break;
        }

        // --- $INDEX_ALLOCATION: обходим все индексные буферы ---
        // Дерево B+ хранит каждую запись ровно один раз, поэтому линейный обход
        // всех буферов даёт полный список — рекурсивный спуск не нужен.
        for &(off, atype) in &attrs {
            if atype != ATTR_INDEX_ALLOCATION || rec[off + 8] == 0 { continue; }
            let alen = rd_u32(rec, off + 4) as usize;
            let run_off = rd_u16(rec, off + 32) as usize;
            if off + run_off > rec.len() || off + alen > rec.len() { continue; }

            let runs = parse_runs(&rec[off + run_off..off + alen]);
            if runs.is_empty() { continue; }

            let real_size = rd_u64(rec, off + 48) as usize;
            if real_size == 0 || real_size > MAX_FILE_SIZE { continue; }
            if block_size < 512 || block_size > 65536 { block_size = 4096; }

            let mut buf = vec![0u8; real_size];
            if self.read_runs(&runs, 0, &mut buf).is_err() { continue; }

            for chunk in buf.chunks_mut(block_size) {
                if chunk.len() < 24 || &chunk[0..4] != b"INDX" { continue; }
                // INDX-записи тоже защищены update sequence array.
                apply_fixup(chunk, self.boot.bytes_per_sector);
                let e_off = rd_u32(chunk, 24) as usize;
                let e_total = rd_u32(chunk, 28) as usize;
                let s = 24 + e_off;
                let e = (24 + e_total).min(chunk.len());
                self.collect_entries(chunk, s, e, &mut out);
            }
        }

        Ok(out)
    }

    /// Разобрать массив индексных записей в диапазоне `[start, end)`.
    fn collect_entries(&self, buf: &[u8], start: usize, end: usize,
                       out: &mut Vec<(String, u64, bool, u64)>) {
        let mut pos = start;
        while pos + 16 <= end && pos + 16 <= buf.len() {
            let file_ref = rd_u64(buf, pos);
            let entry_len = rd_u16(buf, pos + 8) as usize;
            let key_len = rd_u16(buf, pos + 10) as usize;
            let flags = rd_u16(buf, pos + 12);

            if entry_len < 16 || pos + entry_len > buf.len() { break; }
            // flags & 2 = последняя (заглушка) запись, ключа нет.
            if flags & 0x2 != 0 { break; }

            let key_start = pos + 16;
            if key_len > 0 && key_start + key_len <= buf.len() {
                if let Some(info) = parse_name_key(&buf[key_start..key_start + key_len]) {
                    let mft_num = file_ref & 0x0000_FFFF_FFFF_FFFF;
                    let is_dot = info.name == "." || info.name == "..";
                    // namespace 2 = DOS 8.3, не показываем (дубль Win32-имени).
                    if !is_dot && info.namespace != 2 && mft_num != 0 {
                        out.push((info.name, mft_num, info.is_dir, info.size));
                    }
                }
            }

            pos += entry_len;
        }
    }

    /// Найти запись по пути, начиная с корневого каталога (MFT #5).
    fn find_path(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let mut rec = self.read_record(5)?;

        for part in path.split('/').filter(|s| !s.is_empty()) {
            if rd_u16(&rec, 22) & 0x2 == 0 {
                return Err(FsError::NotADirectory);
            }
            let entries = self.list_dir(&rec)?;
            let found = entries.into_iter()
                .find(|(n, _, _, _)| n.eq_ignore_ascii_case(part))
                .ok_or(FsError::NotFound)?;
            rec = self.read_record(found.1)?;
        }

        Ok(rec)
    }
}


/// Открытый файл NTFS (данные в памяти).
pub struct NtfsFile {
    data: Vec<u8>,
    position: u64,
}

impl File for NtfsFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() { return Ok(0); }
        let n = buf.len().min(self.data.len() - pos);
        buf[..n].copy_from_slice(&self.data[pos..pos + n]);
        self.position += n as u64;
        Ok(n)
    }

    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> { Err(FsError::ReadOnly) }

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

impl FileSystem for NtfsFs {
    fn name(&self) -> &str { "ntfs" }

    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let rec = self.find_path(path)?;
        if rd_u16(&rec, 22) & 0x2 != 0 { return Err(FsError::IsADirectory); }
        let data = self.file_data(&rec)?;
        Ok(Box::new(NtfsFile { data, position: 0 }))
    }

    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> { Err(FsError::ReadOnly) }
    fn remove(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let rec = self.find_path(path)?;
        if rd_u16(&rec, 22) & 0x2 == 0 { return Err(FsError::NotADirectory); }

        let entries = self.list_dir(&rec)?;
        let mut result = Vec::with_capacity(entries.len());
        for (name, _num, is_dir, size) in entries {
            let perms = if is_dir { 0o755 } else { 0o644 };
            result.push(DirEntry {
                name,
                file_type: if is_dir { FileType::Directory } else { FileType::File },
                size: size as usize,
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
        let rec = self.find_path(path)?;
        let is_dir = rd_u16(&rec, 22) & 0x2 != 0;

        // Имя: последний компонент пути (у корня — "/").
        let name = match path.rsplit('/').find(|s| !s.is_empty()) {
            Some(s) => String::from(s),
            None => String::from("/"),
        };

        // Размер: из $DATA для файлов, 0 для каталогов.
        let size = if is_dir { 0 } else { self.data_size(&rec) };

        Ok(FileMetadata {
            name,
            file_type: if is_dir { FileType::Directory } else { FileType::File },
            size,
            permissions: Permissions::new(if is_dir { 0o755 } else { 0o644 }),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
        })
    }
}

