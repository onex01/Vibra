// Приёмник boot-лога: FAT32-раздел с меткой VIBRALOG, файл /VIBRA.LOG.
//
// Раздел ищется на всех USB-накопителях (mass storage) и на SATA-дисках
// (нужно для QEMU-теста: второй AHCI-диск). Проверяются и GPT, и MBR —
// в hybrid-образе (`iso-debug`) логический раздел добавлен только в MBR,
// а GPT-запись о нём ничего не знает.
//
// Файл создаётся скриптом сборки заранее и заполнен нулями: цепочка кластеров
// уже выделена, поэтому в рантайме (в том числе в обработчике паники) не
// требуется аллокация кластеров и запись FAT.
//
// Порядок записи: сначала данные, затем размер в записи каталога. Если питание
// пропадёт между ними, файл останется с прежним (меньшим) размером — то есть
// с валидным префиксом лога, а не с «дырой».

use super::{first_cursor, read_chunk, total_len};
use crate::fs::disk::DiskIo;
use crate::fs::fat32::{DirSlot, Fat32Fs};
use crate::fs::partition::{read_partitions, PartitionDisk};
use crate::fs::vfs::FsError;
use crate::println;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

/// Метка тома-приёмника (11 байт BPB, выровнена пробелами).
pub const VOLUME_LABEL: [u8; 11] = *b"VIBRALOG   ";
/// Имя файла лога (8.3).
pub const LOG_FILE: &str = "VIBRA.LOG";
/// Предел, если файл создаётся на месте (а не предвыделен образом).
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
/// Порция записи: фиксированный буфер, без аллокаций (путь паники).
const CHUNK: usize = 4096;

static mut CHUNK_BUF: [u8; CHUNK] = [0; CHUNK];

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
/// Сколько раз пробовали найти раздел. Ограничение — чтобы не сканировать
/// все диски на каждом checkpoint, если VIBRALOG на машине нет.
static ATTACH_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
/// Время последней попытки (мс с загрузки) — троттлинг после boot.
static LAST_ATTACH_MS: AtomicUsize = AtomicUsize::new(0);
/// Во время загрузки попыток немного: диски появляются на шаге 8.5.
const MAX_BOOT_ATTEMPTS: usize = 3;
/// После загрузки — редкие попытки для горячего подключения носителя.
const MAX_IDLE_ATTEMPTS: usize = 4;
/// Интервал между попытками в idle: чаще — значит блокировать shell
/// (скан дисков занимает десятки мс, а PS/2-сканкоды при этом теряются).
const IDLE_RETRY_MS: usize = 30_000;

struct Sink {
    fs: Fat32Fs,
    slot: DirSlot,
    start_cluster: u32,
    /// Сколько байт можно записать в файл (размер предвыделенной цепочки).
    capacity: u64,
    /// Сколько байт уже записано.
    written: u64,
    /// Позиция в кольцевом буфере, до которой лог записан на диск.
    cursor: usize,
}

/// Обёртка `Arc<Mutex<Box<dyn DiskIo>>>` в `DiskIo`: нужна `read_partitions`,
/// которая принимает `&mut dyn DiskIo`.
struct DiskView(Arc<Mutex<Box<dyn DiskIo>>>);

impl DiskIo for DiskView {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        self.0.lock().read(sector, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        self.0.lock().write(sector, buf)
    }

    fn sector_size(&self) -> usize {
        self.0.lock().sector_size()
    }

    fn read_lba(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FsError> {
        self.0.lock().read_lba(lba, buf)
    }
}

/// Записи MBR: `(start_lba, sectors, type)`.
fn mbr_entries(shared: &Arc<Mutex<Box<dyn DiskIo>>>) -> Vec<(u64, u64, u8)> {
    let mut sector = [0u8; 512];
    if shared.lock().read(0, &mut sector).is_err() {
        return Vec::new();
    }
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return Vec::new();
    }

    let mut result = Vec::new();
    for index in 0..4usize {
        let base = 446 + index * 16;
        let kind = sector[base + 4];
        let start = u32::from_le_bytes([
            sector[base + 8],
            sector[base + 9],
            sector[base + 10],
            sector[base + 11],
        ]) as u64;
        let count = u32::from_le_bytes([
            sector[base + 12],
            sector[base + 13],
            sector[base + 14],
            sector[base + 15],
        ]) as u64;
        if count != 0 {
            result.push((start, count, kind));
        }
    }
    result
}

fn is_fat_type(kind: u8) -> bool {
    matches!(kind, 0x01 | 0x04 | 0x06 | 0x0B | 0x0C | 0x0E)
}

fn overlaps(a_start: u64, a_len: u64, b_start: u64, b_len: u64) -> bool {
    a_start < b_start + b_len && b_start < a_start + a_len
}

/// Окна-кандидаты на одном диске: разделы GPT/MBR + hybrid-MBR + весь диск.
fn windows_of(disk: Box<dyn DiskIo>) -> Vec<Box<dyn DiskIo>> {
    let shared = Arc::new(Mutex::new(disk));

    let parts = {
        let mut view = DiskView(shared.clone());
        read_partitions(&mut view)
    };
    let mbr = mbr_entries(&shared);

    let mut windows: Vec<Box<dyn DiskIo>> = Vec::new();
    for part in &parts {
        windows.push(Box::new(PartitionDisk::new(
            shared.clone(),
            part.start_lba,
            part.sectors,
        )));
    }
    for (start, count, kind) in mbr {
        if !is_fat_type(kind) {
            continue;
        }
        // Запись 0xEE —protective MBR, она описывает весь GPT-диск.
        if kind == 0xEE || parts.iter().any(|p| overlaps(p.start_lba, p.sectors, start, count)) {
            continue;
        }
        windows.push(Box::new(PartitionDisk::new(shared.clone(), start, count)));
    }

    if windows.is_empty() {
        // Superfloppy: FAT32 без таблицы разделов (hdd.img в QEMU).
        windows.push(Box::new(DiskView(shared.clone())));
    }
    windows
}

/// Все окна на всех дисках: USB-накопители, затем SATA.
fn candidate_windows() -> Vec<Box<dyn DiskIo>> {
    let mut disks: Vec<Box<dyn DiskIo>> = Vec::new();

    let usb_count = crate::drivers::usb::storage::disks().len();
    for index in 0..usb_count {
        if let Some(disk) = crate::drivers::usb::storage::open(index) {
            disks.push(disk);
        }
    }
    // NVMe: на современных ноутбуках это единственный внутренний диск, и
    // раньше boot-log вообще не сканировал его — VIBRALOG не находился.
    if let Some(disk) = crate::drivers::nvme::create_disk() {
        disks.push(disk);
    }
    for port in crate::drivers::ahci::active_ports() {
        disks.push(Box::new(crate::drivers::ahci::AhciDisk::new(port)));
    }

    let mut windows = Vec::new();
    for disk in disks {
        windows.extend(windows_of(disk));
    }
    windows
}


/// Готовит приёмник: находит/создаёт /VIBRA.LOG и читает его цепочку.
fn prepare(mut fs: Fat32Fs) -> Option<Sink> {
    use crate::fs::vfs::FileSystem;

    let mut entry = match fs.lookup(LOG_FILE) {
        Ok(value) => value,
        Err(_) => return None,
    };
    if entry.is_none() {
        // Файла нет (например, раздел отформатирован вручную) — создаём.
        let _ = FileSystem::create(&mut fs, LOG_FILE);
        entry = fs.lookup(LOG_FILE).ok().flatten();
    }
    let (start, _size, slot) = entry?;

    let clusters = fs.clusters(start).unwrap_or_default();
    let capacity = if clusters.is_empty() {
        // Цепочка пустая — кластеры будут выделяться по мере записи.
        MAX_LOG_BYTES
    } else {
        (clusters.len() as u64 * fs.cluster_size() as u64).min(MAX_LOG_BYTES)
    };

    Some(Sink {
        fs,
        slot,
        start_cluster: start,
        capacity,
        written: 0,
        cursor: 0,
    })
}

/// Ищет раздел VIBRALOG и подключает приёмник.
///
/// Вызывается явно на шаге 8.5 (после enumeration USB) и лениво из
/// `checkpoint()`. Если носителей ещё нет (boot только начался), попытка не
/// расходуется — иначе однократный «not found» на шаге 4 навсегда выключал бы
/// запись на флешку.
pub fn attach() -> bool {
    if SINK.lock().is_some() {
        return true;
    }

    // После загрузки idle-цикл shell дергает `flush` раз в секунду: сканировать
    // диски каждый раз нельзя (долгие блокирующие чтения теряют PS/2-сканкоды),
    // поэтому троттлим ВСЕГДА, а не только после исчерпания boot-попыток.
    // Раньше условие `attempts >= MAX_BOOT_ATTEMPTS` означало, что если за
    // время boot диски ещё не появились и счётчик остался нулевым, то после
    // загрузки `attach()` сканировал носители на каждом `checkpoint()` —
    // то есть раз в секунду и на каждое нажатие клавиши в простое.
    let after_boot = super::boot_finished();
    if after_boot {
        let attempts = ATTACH_ATTEMPTS.load(Ordering::Relaxed);
        if attempts >= MAX_BOOT_ATTEMPTS + MAX_IDLE_ATTEMPTS {
            return false;
        }
        let now = crate::timer::uptime_ms() as usize;
        let last = LAST_ATTACH_MS.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < IDLE_RETRY_MS {
            return false;
        }
    }
    LAST_ATTACH_MS.store(crate::timer::uptime_ms() as usize, Ordering::Relaxed);

    // Счётчик попыток увеличиваем ДО сканирования: если дисков ещё нет,
    // иначе каждый checkpoint заново перебирал бы пустой список.
    let attempt = ATTACH_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    if attempt >= MAX_BOOT_ATTEMPTS + MAX_IDLE_ATTEMPTS {
        return false;
    }
    if !after_boot && attempt >= MAX_BOOT_ATTEMPTS {
        return false;
    }

    let windows = candidate_windows();
    if windows.is_empty() {
        return false;
    }

    for window in windows {
        let fs = match Fat32Fs::new_rw(window) {
            Ok(fs) => fs,
            Err(error) => {
                if !after_boot {
                    println!("[LOG] candidate volume rejected: {:?}", error);
                }
                continue;
            }
        };
        let label = fs.label();
        if label != VOLUME_LABEL {
            // После загрузки не шумим: сообщение повторялось бы на каждой
            // попытке и засоряло serial/консоль во время ввода в shell.
            if !after_boot {
                println!(
                    "[LOG] skipping volume labeled {:?} (want {:?})",
                    core::str::from_utf8(&label).unwrap_or("?"),
                    core::str::from_utf8(&VOLUME_LABEL).unwrap_or("?")
                );
            }
            continue;
        }

        let mut sink = match prepare(fs) {
            Some(sink) => sink,
            None => {
                println!("[LOG] VIBRALOG found, but /{} is unusable", LOG_FILE);
                continue;
            }
        };

        // Всё, что накопилось до появления USB, уходит на диск сразу.
        sink.cursor = first_cursor();
        let capacity = sink.capacity;
        *SINK.lock() = Some(sink);
        println!(
            "[LOG] writing boot log to /{} on VIBRALOG ({} KiB slot)",
            LOG_FILE,
            capacity / 1024
        );
        flush(false);
        return true;
    }

    if !after_boot {
        println!("[LOG] no FAT32 volume labeled VIBRALOG found");
    }
    false
}

/// Сбросить на диск всё, что ещё не записано.
///
/// `panic` = true: без аллокаций и не больше одной порции (обработчик паники
/// не должен зависнуть на длинной записи).
pub fn flush(panic: bool) {
    let mut guard = match SINK.try_lock() {
        Some(guard) => guard,
        None => return,
    };
    let Some(sink) = guard.as_mut() else {
        drop(guard);
        if !panic {
            attach();
        }
        return;
    };

    write_pending(sink, panic);
}

fn write_pending(sink: &mut Sink, panic: bool) {
    let buffer: &mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(CHUNK_BUF) as *mut u8, CHUNK)
    };

    // Кольцо успело перезаписать непрочитанное — фиксируем пропуск.
    let first = first_cursor();
    if sink.cursor < first {
        let marker = alloc::format!(
            "\n[LOG] {} byte(s) dropped: ring buffer overflowed\n",
            first - sink.cursor
        );
        let _ = write_bytes(sink, marker.as_bytes());
        sink.cursor = first;
    }

    let mut batches = 0;
    loop {
        let (count, next) = read_chunk(sink.cursor, buffer);
        if count == 0 {
            break;
        }
        if !write_bytes(sink, &buffer[..count]) {
            break;
        }
        sink.cursor = next;
        batches += 1;
        if panic && batches >= 1 {
            break;
        }
    }

    let _ = total_len();
}

/// Записывает порцию лога; `false` — запись прекращена (переполнение/ошибка).
fn write_bytes(sink: &mut Sink, data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }

    if sink.written + data.len() as u64 > sink.capacity {
        let marker = b"\n[LOG] log file is full; output truncated\n";
        if sink.written + marker.len() as u64 <= sink.capacity
            && sink
                .fs
                .write_at(&mut sink.start_cluster, sink.written, marker)
                .is_ok()
        {
            sink.written += marker.len() as u64;
            let _ = sink
                .fs
                .publish_size(&sink.slot, sink.start_cluster, sink.written as u32);
        }
        return false;
    }

    match sink
        .fs
        .write_at(&mut sink.start_cluster, sink.written, data)
    {
        Ok(_) => {
            sink.written += data.len() as u64;
            // Размер публикуем ПОСЛЕ данных (см. комментарий в шапке).
            let _ = sink
                .fs
                .publish_size(&sink.slot, sink.start_cluster, sink.written as u32);
            true
        }
        Err(_) => false,
    }
}

/// Сколько байт записано в файл лога.
pub fn written() -> u64 {
    SINK.lock().as_ref().map_or(0, |sink| sink.written)
}

/// Подключён ли приёмник.
pub fn is_attached() -> bool {
    SINK.lock().is_some()
}
