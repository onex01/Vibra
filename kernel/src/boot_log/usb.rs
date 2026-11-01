// Приёмник логов на FAT32: boot-лог (VIBRA.LOG) и журнал shell (SHELL.LOG).
//
// Раздел ищется в таком порядке:
//   1. том с меткой `VIBRALOG` — явно выделенный раздел логов;
//   2. загрузочный том — тот, где лежит `KERNEL.ELF` (или
//      `EFI/BOOT/BOOTX64.EFI`). Так лог появляется даже на машине, где
//      никто не готовил VIBRALOG: снял загрузочный носитель — прочитал лог.
//
// Проверяются все диски: USB mass storage, NVMe, AHCI (SATA), ATAPI (CD).
// Для hybrid-образов (`iso-debug`) дополнительно смотрятся MBR-записи, даже
// если GPT о них ничего не знает.
//
// Файлы создаются заранее скриптом сборки либо на месте. Порядок записи:
// сначала данные, затем размер в записи каталога — при обрыве питания файл
// остаётся с валидным префиксом лога, а не с «дырой».

use super::{first_cursor, read_chunk, session_first_cursor, session_read_chunk};
use crate::fs::disk::DiskIo;
use crate::fs::fat32::{DirSlot, Fat32Fs};
use crate::fs::partition::{read_partitions, PartitionDisk};
use crate::fs::vfs::{FileSystem, FsError};
use crate::println;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

/// Метка тома-приёмника (11 байт BPB, выровнена пробелами).
pub const VOLUME_LABEL: [u8; 11] = *b"VIBRALOG   ";
/// Имя файла boot-лога (8.3).
pub const LOG_FILE: &str = "VIBRA.LOG";
/// Имя файла журнала shell (8.3).
pub const SHELL_FILE: &str = "SHELL.LOG";
/// Маркер загрузочного тома: ядро, которое сейчас исполняется.
const KERNEL_FILE: &str = "KERNEL.ELF";
/// Второй маркер загрузочного тома (UEFI-загрузчик Limine).
const ESP_MARKER: &str = "EFI/BOOT/BOOTX64.EFI";

/// Предел boot-лога на носителе.
const MAX_BOOT_BYTES: u64 = 4 * 1024 * 1024;
/// Предел журнала shell на носителе.
const MAX_SHELL_BYTES: u64 = 2 * 1024 * 1024;
/// Порция записи: фиксированный буфер, без аллокаций (путь паники).
const CHUNK: usize = 4096;

static mut CHUNK_BUF: [u8; CHUNK] = [0; CHUNK];

/// Куда пишется лог.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Раздел с меткой VIBRALOG.
    LogVolume,
    /// Загрузочный том (рядом с KERNEL.ELF).
    BootVolume,
}

impl Target {
    fn as_str(self) -> &'static str {
        match self {
            Target::LogVolume => "VIBRALOG",
            Target::BootVolume => "boot volume",
        }
    }
}

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
    /// true — журнал shell, false — boot-лог.
    shell: bool,
}

static BOOT_SINK: Mutex<Option<Sink>> = Mutex::new(None);
static SHELL_SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Найденный том и его роль (для `log sink` / `log scan`).
static VOLUME_TARGET: Mutex<Option<Target>> = Mutex::new(None);
static VOLUME_LABEL_FOUND: Mutex<Option<String>> = Mutex::new(None);

/// Сколько раз пробовали найти раздел. Ограничение — чтобы не сканировать
/// все диски на каждом checkpoint, если носителя нет.
static ATTACH_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
/// Время последней попытки (мс с загрузки) — троттлинг после boot.
static LAST_ATTACH_MS: AtomicUsize = AtomicUsize::new(0);
/// Уже сообщили, что носителя нет (чтобы не спамить в лог).
static REPORTED_MISSING: AtomicBool = AtomicBool::new(false);

/// Во время загрузки попыток немного: диски появляются на шаге 8.5.
const MAX_BOOT_ATTEMPTS: usize = 6;
/// Интервал между попытками после загрузки. Приёмник больше не «сдаётся»:
/// флешку можно воткнуть позже, и лог продолжит писаться.
const IDLE_RETRY_MS: usize = 10_000;

/// Окно в диск: общий носитель + диапазон в 512-байтных секторах.
///
/// Хранится как описание, а не как готовый `DiskIo`: для двух файлов
/// (boot + shell) нужно два независимых окна на один и тот же том.
#[derive(Clone)]
struct Window {
    shared: Arc<Mutex<Box<dyn DiskIo>>>,
    offset: u64,
    /// 0 — весь диск (superfloppy без таблицы разделов).
    sectors: u64,
}

impl Window {
    fn open(&self) -> Box<dyn DiskIo> {
        if self.sectors == 0 {
            Box::new(DiskView(self.shared.clone()))
        } else {
            Box::new(PartitionDisk::new(
                self.shared.clone(),
                self.offset,
                self.sectors,
            ))
        }
    }
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

    fn is_read_only(&self) -> bool {
        self.0.lock().is_read_only()
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
fn windows_of(disk: Box<dyn DiskIo>) -> Vec<Window> {
    let shared = Arc::new(Mutex::new(disk));

    let parts = {
        let mut view = DiskView(shared.clone());
        read_partitions(&mut view)
    };
    let mbr = mbr_entries(&shared);

    let mut windows: Vec<Window> = Vec::new();
    for part in &parts {
        windows.push(Window {
            shared: shared.clone(),
            offset: part.start_lba,
            sectors: part.sectors,
        });
    }
    for (start, count, kind) in mbr {
        if !is_fat_type(kind) {
            continue;
        }
        // Запись 0xEE — protective MBR, она описывает весь GPT-диск.
        if kind == 0xEE || parts.iter().any(|p| overlaps(p.start_lba, p.sectors, start, count)) {
            continue;
        }
        windows.push(Window {
            shared: shared.clone(),
            offset: start,
            sectors: count,
        });
    }

    if windows.is_empty() {
        // Superfloppy: FAT32 без таблицы разделов (hdd.img в QEMU).
        windows.push(Window {
            shared,
            offset: 0,
            sectors: 0,
        });
    }
    windows
}

/// Все окна на всех дисках: USB-накопители, NVMe, ATAPI, SATA.
fn candidate_windows() -> Vec<Window> {
    let mut disks: Vec<Box<dyn DiskIo>> = Vec::new();

    let usb_count = crate::drivers::usb::storage::disks().len();
    for index in 0..usb_count {
        if let Some(disk) = crate::drivers::usb::storage::open(index) {
            disks.push(disk);
        }
    }
    // NVMe: на современных ноутбуках это единственный внутренний диск.
    if let Some(disk) = crate::drivers::nvme::create_disk() {
        disks.push(disk);
    }
    // ATAPI CD/DVD: debug ISO держит логи на appended-разделе.
    for port in crate::drivers::ahci::atapi_ports() {
        if let Some(disk) = crate::drivers::ahci::AtapiDisk::new(port) {
            disks.push(Box::new(disk));
        }
    }
    for port in crate::drivers::ahci::active_ports() {
        disks.push(Box::new(crate::drivers::ahci::AhciDisk::new(port)));
    }
    for index in 0..crate::drivers::ide::disk_count() {
        if let Some(disk) = crate::drivers::ide::open(index) {
            disks.push(disk);
        }
    }

    let mut windows = Vec::new();
    for disk in disks {
        windows.extend(windows_of(disk));
    }
    windows
}

/// Том подходит как загрузочный? Ищем ядро или загрузчик Limine.
fn looks_like_boot_volume(fs: &Fat32Fs) -> bool {
    fs.lookup(KERNEL_FILE).map(|entry| entry.is_some()).unwrap_or(false)
        || fs
            .lookup(ESP_MARKER)
            .map(|entry| entry.is_some())
            .unwrap_or(false)
}

/// Готовит приёмник: находит/создаёт файл и читает его цепочку.
fn prepare(mut fs: Fat32Fs, file: &str, shell: bool) -> Option<Sink> {
    let mut entry = match fs.lookup(file) {
        Ok(value) => value,
        Err(_) => return None,
    };
    if entry.is_none() {
        // Файла нет (например, раздел отформатирован вручную) — создаём.
        let _ = FileSystem::create(&mut fs, file);
        entry = fs.lookup(file).ok().flatten();
    }
    let (start, _size, slot) = entry?;

    let limit = if shell { MAX_SHELL_BYTES } else { MAX_BOOT_BYTES };
    let clusters = fs.clusters(start).unwrap_or_default();
    let capacity = if clusters.is_empty() {
        // Цепочка пустая — кластеры будут выделяться по мере записи.
        limit
    } else {
        (clusters.len() as u64 * fs.cluster_size() as u64).min(limit)
    };

    Some(Sink {
        fs,
        slot,
        start_cluster: start,
        capacity,
        written: 0,
        cursor: 0,
        shell,
    })
}

/// Ищет том для логов и подключает оба приёмника.
///
/// Вызывается явно на шаге 8.5 (после enumeration USB) и лениво из
/// `checkpoint()`. Если носителей ещё нет (boot только начался), попытка не
/// расходуется — иначе однократный «not found» на шаге 4 навсегда выключал бы
/// запись на флешку.
pub fn attach() -> bool {
    if BOOT_SINK.lock().is_some() {
        return true;
    }

    let after_boot = super::boot_finished();
    if after_boot {
        let now = crate::timer::uptime_ms() as usize;
        let last = LAST_ATTACH_MS.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < IDLE_RETRY_MS {
            return false;
        }
    }
    LAST_ATTACH_MS.store(crate::timer::uptime_ms() as usize, Ordering::Relaxed);

    let attempt = ATTACH_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    if !after_boot && attempt >= MAX_BOOT_ATTEMPTS {
        return false;
    }

    let windows = candidate_windows();
    if windows.is_empty() {
        report_missing("no block devices visible yet");
        return false;
    }

    // Первый проход: ищем VIBRALOG. Второй: любой загрузочный том.
    for target in [Target::LogVolume, Target::BootVolume] {
        for window in &windows {
            let fs = match Fat32Fs::new_rw(window.open()) {
                Ok(fs) => fs,
                Err(_) => continue,
            };
            if fs.device_is_read_only() {
                continue;
            }

            let label = fs.label();
            let matches = match target {
                Target::LogVolume => label == VOLUME_LABEL,
                Target::BootVolume => looks_like_boot_volume(&fs),
            };
            if !matches {
                continue;
            }

            let Some(boot_sink) = prepare(fs, LOG_FILE, false) else {
                println!("[LOG] {} found, but /{} is unusable", target.as_str(), LOG_FILE);
                continue;
            };

            // Журнал shell — отдельный экземпляр ФС на том же окне.
            let shell_sink = match Fat32Fs::new_rw(window.open()) {
                Ok(fs) => prepare(fs, SHELL_FILE, true),
                Err(_) => None,
            };

            let capacity = boot_sink.capacity;
            let mut sink = boot_sink;
            // Всё, что накопилось до появления носителя, уходит на диск сразу.
            sink.cursor = first_cursor();
            *BOOT_SINK.lock() = Some(sink);

            let shell_ok = if let Some(mut shell_sink) = shell_sink {
                shell_sink.cursor = session_first_cursor();
                *SHELL_SINK.lock() = Some(shell_sink);
                true
            } else {
                false
            };

            *VOLUME_TARGET.lock() = Some(target);
            *VOLUME_LABEL_FOUND.lock() =
                Some(String::from_utf8_lossy(&label).trim().to_string());

            println!(
                "[LOG] writing logs to {} ({}): /{} ({} KiB slot), /{}: {}",
                target.as_str(),
                String::from_utf8_lossy(&label).trim(),
                LOG_FILE,
                capacity / 1024,
                SHELL_FILE,
                if shell_ok { "on" } else { "unavailable" }
            );

            REPORTED_MISSING.store(false, Ordering::Relaxed);
            flush(false);
            return true;
        }
    }

    report_missing("no FAT32 volume with VIBRALOG or KERNEL.ELF");
    false
}

/// Принудительная попытка подключения (команда `log attach`).
///
/// Сбрасывает троттлинг и счётчики: пользователь мог только что воткнуть
/// флешку, и ждать 10 секунд незачем.
pub fn force_attach() -> bool {
    LAST_ATTACH_MS.store(0, Ordering::Relaxed);
    ATTACH_ATTEMPTS.store(0, Ordering::Relaxed);
    REPORTED_MISSING.store(false, Ordering::Relaxed);
    attach()
}

/// Сообщить об отсутствии приёмника один раз, чтобы не засорять лог.
fn report_missing(reason: &str) {
    if REPORTED_MISSING.swap(true, Ordering::Relaxed) {
        return;
    }
    // Во время загрузки диски появляются позже (шаг 8.5), поэтому не пугаем
    // «not found» до входа в shell: сообщение всё равно было бы ложным.
    if !super::boot_finished() {
        return;
    }
    println!("[LOG] no log volume found: {}", reason);
    println!("[LOG] create a FAT32 volume labeled VIBRALOG, or boot from media with KERNEL.ELF");
}

/// Диагностика для `log scan`: что видно и почему не подошло.
pub fn scan() -> Vec<String> {
    let mut lines = Vec::new();
    let windows = candidate_windows();
    lines.push(format!("candidates: {}", windows.len()));

    for (index, window) in windows.iter().enumerate() {
        let fs = match Fat32Fs::new_rw(window.open()) {
            Ok(fs) => fs,
            Err(error) => {
                lines.push(format!("  [{}] not FAT32 ({:?})", index, error));
                continue;
            }
        };
        let label = String::from_utf8_lossy(&fs.label()).trim().to_string();
        let read_only = fs.device_is_read_only();
        let boot = looks_like_boot_volume(&fs);
        let is_log = fs.label() == VOLUME_LABEL;
        let boot_file = fs.lookup(LOG_FILE).ok().flatten().map(|(_, size, _)| size);
        let shell_file = fs.lookup(SHELL_FILE).ok().flatten().map(|(_, size, _)| size);

        lines.push(format!(
            "  [{}] label=\"{}\" {} boot={} log={} VIBRA.LOG={} SHELL.LOG={}",
            index,
            label,
            if read_only { "ro" } else { "rw" },
            boot,
            is_log,
            boot_file.map(|s| format!("{} B", s)).unwrap_or_else(|| "-".to_string()),
            shell_file.map(|s| format!("{} B", s)).unwrap_or_else(|| "-".to_string()),
        ));
    }
    lines
}

/// Текущее состояние приёмника для `log sink`.
pub fn describe() -> Vec<String> {
    let mut lines = Vec::new();
    match *VOLUME_TARGET.lock() {
        Some(target) => {
            let label = VOLUME_LABEL_FOUND
                .lock()
                .clone()
                .unwrap_or_else(|| String::from("?"));
            lines.push(format!("volume: {} (label \"{}\")", target.as_str(), label));
        }
        None => lines.push(String::from("volume: not attached")),
    }

    if let Some(sink) = BOOT_SINK.lock().as_ref() {
        lines.push(format!(
            "boot log: {} written, {} KiB capacity",
            sink.written,
            sink.capacity / 1024
        ));
    } else {
        lines.push(String::from("boot log: not attached"));
    }

    if let Some(sink) = SHELL_SINK.lock().as_ref() {
        lines.push(format!(
            "shell log: {} written, {} KiB capacity",
            sink.written,
            sink.capacity / 1024
        ));
    } else {
        lines.push(String::from("shell log: not attached"));
    }

    lines
}

/// Сбросить на диск всё, что ещё не записано.
///
/// `panic` = true: без аллокаций и не больше одной порции (обработчик паники
/// не должен зависнуть на длинной записи).
pub fn flush(panic: bool) {
    flush_sink(&BOOT_SINK, panic, false);
    flush_sink(&SHELL_SINK, panic, true);
}

fn flush_sink(slot: &Mutex<Option<Sink>>, panic: bool, shell: bool) {
    let mut guard = match slot.try_lock() {
        Some(guard) => guard,
        None => return,
    };
    let Some(sink) = guard.as_mut() else {
        drop(guard);
        if !panic && !shell {
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

    let (first, read) : (usize, fn(usize, &mut [u8]) -> (usize, usize)) = if sink.shell {
        (session_first_cursor(), session_read_chunk)
    } else {
        (first_cursor(), read_chunk)
    };

    // Кольцо успело перезаписать непрочитанное — фиксируем пропуск.
    if sink.cursor < first {
        let marker = format!(
            "\n[LOG] {} byte(s) dropped: ring buffer overflowed\n",
            first - sink.cursor
        );
        let _ = write_bytes(sink, marker.as_bytes());
        sink.cursor = first;
    }

    let mut batches = 0;
    loop {
        let (count, next) = read(sink.cursor, buffer);
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

/// Сколько байт boot-лога записано в файл.
pub fn written() -> u64 {
    BOOT_SINK.lock().as_ref().map_or(0, |sink| sink.written)
}

/// Сколько байт журнала shell записано в файл.
pub fn shell_written() -> u64 {
    SHELL_SINK.lock().as_ref().map_or(0, |sink| sink.written)
}

/// Подключён ли приёмник.
pub fn is_attached() -> bool {
    BOOT_SINK.lock().is_some()
}

/// Подключён ли приёмник журнала shell.
pub fn is_shell_attached() -> bool {
    SHELL_SINK.lock().is_some()
}
