// Boot Log — кольцевой буфер в памяти + сохранение на носитель.
//
// Все `println!` (serial) попадают в кольцевой буфер (128 КиБ) через
// `log_byte`. Оттуда лог:
//   1. пишется в /var/log/boot.log (ramfs, после init_filesystem);
//   2. в файл /VIBRA.LOG на FAT32-разделе с меткой `VIBRALOG` (загрузочная
//      флешка или отдельный диск). Это единственный канал диагностики на
//      ноутбуке без COM-порта. Раньше приёмник собирался только с feature
//      `boot-log-to-usb`, и в обычной сборке лог жил лишь в ramfs — то есть
//      исчезал при выключении. Теперь модуль компилируется всегда;
//   3. при feature `boot-log-to-disk` — legacy raw-запись в LBA 2/3 (опасно
//      на реальном железе, только для тестового носителя).
//
// Кольцо нужно потому, что запись на USB появляется не сразу: контроллер
// инициализируется на шаге 7, перечисление — на шаге 8.5. Всё, что было
// напечатано раньше, при `attach()` уходит на диск целиком.

use crate::println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Boot-фаза завершена (`init()` вызван). До этого `flush` может повторно
/// искать VIBRALOG, после — только изредка: idle-цикл shell не должен
/// сканировать диски каждую секунду.
static BOOT_DONE: AtomicBool = AtomicBool::new(false);

/// Размер кольцевого буфера. 128 КиБ — весь boot-лог с запасом;
/// старые 32 КиБ обрезали лог на этапе enumeration USB.
pub const LOG_SIZE: usize = 128 * 1024;

static mut LOG_BUF: [u8; LOG_SIZE] = [0; LOG_SIZE];
/// Сколько байт всего записано (монотонно). Позиция в кольце = total % LOG_SIZE.
static LOG_TOTAL: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "boot-log-to-disk")]
static LOG_WRITTEN: AtomicBool = AtomicBool::new(false);

pub mod usb;

/// Записать байт в лог-буфер.
pub fn log_byte(b: u8) {
    let position = LOG_TOTAL.fetch_add(1, Ordering::Relaxed) % LOG_SIZE;
    unsafe {
        LOG_BUF[position] = b;
    }
}

/// Сколько байт всего записано в лог.
pub fn total_len() -> usize {
    LOG_TOTAL.load(Ordering::Relaxed)
}

/// Начало валидного окна: всё, что было записано раньше, уже перезаписано.
pub fn first_cursor() -> usize {
    total_len().saturating_sub(LOG_SIZE)
}

/// Boot-фаза завершена? См. `BOOT_DONE`.
pub fn boot_finished() -> bool {
    BOOT_DONE.load(Ordering::Relaxed)
}

/// Отметить конец boot-фазы (вызывается при входе в shell).
pub fn mark_boot_finished() {
    BOOT_DONE.store(true, Ordering::Relaxed);
}

/// Копирует до `out.len()` байт начиная с `cursor`.
/// Возвращает `(скопировано, новый курсор)`.
pub fn read_chunk(cursor: usize, out: &mut [u8]) -> (usize, usize) {
    let total = total_len();
    let mut start = cursor.max(first_cursor());
    if start >= total || out.is_empty() {
        return (0, total);
    }

    let index = start % LOG_SIZE;
    let count = (LOG_SIZE - index).min(total - start).min(out.len());
    unsafe {
        out[..count].copy_from_slice(&LOG_BUF[index..index + count]);
    }
    start += count;
    (count, start)
}

/// Копирует весь лог в `out` (дописывая).
pub fn copy_log(out: &mut Vec<u8>) {
    let mut cursor = first_cursor();
    loop {
        let total = total_len();
        if cursor >= total {
            break;
        }
        let index = cursor % LOG_SIZE;
        let count = (LOG_SIZE - index).min(total - cursor);
        unsafe {
            out.extend_from_slice(&LOG_BUF[index..index + count]);
        }
        cursor += count;
    }
}

/// Весь лог как вектор (для файлов и serial).
pub fn get_log_bytes() -> Vec<u8> {
    let mut data = Vec::with_capacity(LOG_SIZE);
    copy_log(&mut data);
    data
}

/// Legacy raw-запись лога в LBA 2/3 первого SATA-порта.
///
/// По умолчанию выключено. На ноутбуке первый AHCI-порт — это внутренний
/// SSD, и LBA 2 попадает в GPT partition entries: загрузка с USB-образа
/// портила таблицу разделов. Только `boot-log-to-disk` и тестовый носитель.
#[cfg(feature = "boot-log-to-disk")]
fn checkpoint_raw_disk() {
    let log = get_log_bytes();
    if log.is_empty() {
        return;
    }
    let Some(port) = crate::drivers::ahci::first_port() else {
        return;
    };

    let mut head = [0u8; 512];
    let n = log.len().min(512);
    head[..n].copy_from_slice(&log[..n]);
    let _ = crate::drivers::ahci::write_sectors(port, 2, 1, &head);

    let mut tail = [0u8; 512];
    let start = log.len().saturating_sub(512);
    let tail_bytes = &log[start..];
    let dst = 512 - tail_bytes.len();
    tail[dst..].copy_from_slice(tail_bytes);
    let _ = crate::drivers::ahci::write_sectors(port, 3, 1, &tail);
}

#[cfg(not(feature = "boot-log-to-disk"))]
fn checkpoint_raw_disk() {}

/// Сбросить накопленный лог на носители (checkpoint).
///
/// Вызывается на ключевых шагах boot, раз в секунду из idle-цикла shell
/// и из обработчика паники (`panic_flush`).
pub fn checkpoint() {
    checkpoint_raw_disk();
    usb::flush(false);
}

/// Запись лога при панике: без аллокаций, максимум одна порция.
pub fn panic_flush() {
    usb::flush(true);
}

/// Legacy raw-запись (до init_filesystem). См. `checkpoint_raw_disk`.
#[cfg(feature = "boot-log-to-disk")]
pub fn flush_raw() {
    if LOG_WRITTEN.load(Ordering::Relaxed) {
        return;
    }
    if total_len() == 0 {
        return;
    }
    let Some(port) = crate::drivers::ahci::first_port() else {
        return;
    };

    let log = get_log_bytes();
    let mut sector = [0u8; 512];
    let copy_len = log.len().min(512);
    sector[..copy_len].copy_from_slice(&log[..copy_len]);
    if crate::drivers::ahci::write_sectors(port, 2, 1, &sector) {
        LOG_WRITTEN.store(true, Ordering::Relaxed);
    }
}

#[cfg(not(feature = "boot-log-to-disk"))]
pub fn flush_raw() {}

/// Сохранить лог в файл ramfs (после init_filesystem).
pub fn flush_to_file(path: &str) {
    let log = get_log_bytes();
    if log.is_empty() {
        return;
    }

    // write_file сам создаёт/перезаписывает файл через VFS.
    let _ = crate::fs::write_file(path, &log);
}

/// Инициализация: raw-запись + файлы в ramfs + сводка в serial.
///
/// Запись на USB запускается раньше, из `boot_log::usb::attach()` —
/// сразу после того, как найден накопитель.
pub fn init() {
    flush_raw();

    flush_to_file("/var/log/boot.log");

    let len = total_len();
    println!("[LOG] Boot log saved ({} bytes)", len);
    println!("[LOG] /var/log/boot.log");

    // Полный лог уже транслировался в serial/framebuffer построчно. Повторная
    // печать всех 128 КиБ на реальном UART занимает десятки секунд, поэтому
    // она доступна только по запросу: `log` или `log <N>` в shell.
}
