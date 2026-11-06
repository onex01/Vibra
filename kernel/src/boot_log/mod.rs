// Boot Log — кольцевой буфер в памяти + сохранение на носитель.
//
// Все `println!` (serial) попадают в кольцевой буфер через `log_byte`.
// Оттуда лог:
//   1. пишется в /var/log/boot.log (ramfs, после init_filesystem);
//   2. в файл VIBRA.LOG на FAT32-разделе с меткой `VIBRALOG` либо, если
//      такого нет, на загрузочном томе (там, где лежит KERNEL.ELF). Это
//      единственный канал диагностики на машине без COM-порта: снял
//      загрузочный носитель — прочитал лог;
//   3. при feature `boot-log-to-disk` — legacy raw-запись в LBA 2/3 (опасно
//      на реальном железе, только для тестового носителя).
//
// Отдельно ведётся журнал shell: всё, что печатается в консоль после старта
// оболочки (prompt, команды, их вывод), копируется в собственное кольцо и
// сохраняется как /var/log/shell.log и SHELL.LOG на носителе. Это позволяет
// восстановить полный сеанс терминала, а не только загрузку.
//
// Кольца нужны потому, что запись на USB появляется не сразу: контроллер
// инициализируется на шаге 7, перечисление — на шаге 8.5. Всё, что было
// напечатано раньше, при `attach()` уходит на диск целиком.

use crate::println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Boot-фаза завершена (`mark_boot_finished`). До этого `flush` может
/// повторно искать приёмник, после — только с троттлингом.
static BOOT_DONE: AtomicBool = AtomicBool::new(false);

/// Размер кольцевого буфера boot-лога.
///
/// 256 КиБ: на реальной машине лог с ACPI, PCI-инвентарём, USB-перечислением
/// и ФС занимает десятки килобайт, а при `diag`-командах легко перерастает
/// прежние 128 КиБ — тогда начало лога терялось.
pub const LOG_SIZE: usize = 256 * 1024;

static mut LOG_BUF: [u8; LOG_SIZE] = [0; LOG_SIZE];
/// Сколько байт всего записано (монотонно). Позиция в кольце = total % LOG_SIZE.
static LOG_TOTAL: AtomicUsize = AtomicUsize::new(0);

/// Размер кольца журнала shell.
pub const SHELL_LOG_SIZE: usize = 64 * 1024;

static mut SHELL_BUF: [u8; SHELL_LOG_SIZE] = [0; SHELL_LOG_SIZE];
static SHELL_TOTAL: AtomicUsize = AtomicUsize::new(0);
/// Захват журнала shell включается при входе в оболочку.
static SHELL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Сколько байт boot-лога уже записано в /var/log/boot.log (ramfs).
static RAMFS_BOOT_WRITTEN: AtomicUsize = AtomicUsize::new(0);
/// Сколько байт журнала shell уже записано в /var/log/shell.log (ramfs).
static RAMFS_SHELL_WRITTEN: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "boot-log-to-disk")]
static LOG_WRITTEN: AtomicBool = AtomicBool::new(false);

pub mod usb;

// ==========================================
// Boot-лог
// ==========================================

/// Записать байт в boot-лог.
pub fn log_byte(b: u8) {
    let position = LOG_TOTAL.fetch_add(1, Ordering::Relaxed) % LOG_SIZE;
    unsafe {
        LOG_BUF[position] = b;
    }
}

/// Сколько байт всего записано в boot-лог.
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

/// Копирует до `out.len()` байт boot-лога начиная с `cursor`.
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

/// Копирует весь boot-лог в `out` (дописывая).
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

/// Весь boot-лог как вектор (для файлов и serial).
pub fn get_log_bytes() -> Vec<u8> {
    let mut data = Vec::with_capacity(LOG_SIZE);
    copy_log(&mut data);
    data
}

// ==========================================
// Журнал shell
// ==========================================

/// Включить захват журнала shell (вызывается при входе в оболочку).
pub fn session_begin() {
    SHELL_ACTIVE.store(true, Ordering::Relaxed);
}

/// Захват журнала shell включён?
pub fn session_active() -> bool {
    SHELL_ACTIVE.load(Ordering::Relaxed)
}

/// Редактор строки перерисовывает ввод через `\x08`-последовательности.
/// На время чтения строки журналы глушатся, а сама команда пишется в журнал
/// целиком (`session_line`) — иначе транскрипт превращается в кашу из
/// «l ls ls ls /».
static ECHO_QUIET: AtomicBool = AtomicBool::new(false);

/// Глушить запись консольного эха в журналы (редактор строки).
pub fn session_suspend() {
    ECHO_QUIET.store(true, Ordering::Relaxed);
}

/// Вернуть запись консоли в журналы.
pub fn session_resume() {
    ECHO_QUIET.store(false, Ordering::Relaxed);
}

/// Эхо сейчас подавлено?
pub fn echo_quiet() -> bool {
    ECHO_QUIET.load(Ordering::Relaxed)
}

/// Записать готовую строку в журнал shell (команда после Enter).
pub fn session_line(text: &str) {
    if !SHELL_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    for byte in text.bytes() {
        let position = SHELL_TOTAL.fetch_add(1, Ordering::Relaxed) % SHELL_LOG_SIZE;
        unsafe {
            SHELL_BUF[position] = byte;
        }
    }
}

/// Записать символ консоли в журнал shell.
///
/// Не-ASCII символы (рамки `tree`, блоки) переводятся в ASCII-эквиваленты:
/// журнал — текстовый файл, а CP437-графика в нём бесполезна.
pub fn session_char(ch: char) {
    if !SHELL_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let mapped = match ch {
        '\u{2500}' | '\u{2014}' | '\u{2013}' => b'-',
        '\u{2502}' => b'|',
        '\u{250C}' | '\u{2510}' | '\u{2514}' | '\u{2518}' => b'+',
        '\u{251C}' | '\u{2524}' | '\u{252C}' | '\u{2534}' | '\u{253C}' => b'+',
        '\u{2588}' | '\u{2591}' | '\u{2592}' | '\u{2593}' | '\u{25A0}' => b'#',
        _ if ch.is_ascii() => ch as u8,
        _ => b'?',
    };
    let position = SHELL_TOTAL.fetch_add(1, Ordering::Relaxed) % SHELL_LOG_SIZE;
    unsafe {
        SHELL_BUF[position] = mapped;
    }
}

/// Сколько байт всего записано в журнал shell.
pub fn session_total() -> usize {
    SHELL_TOTAL.load(Ordering::Relaxed)
}

/// Начало валидного окна журнала shell.
pub fn session_first_cursor() -> usize {
    session_total().saturating_sub(SHELL_LOG_SIZE)
}

/// Читает порцию журнала shell: `(скопировано, новый курсор)`.
pub fn session_read_chunk(cursor: usize, out: &mut [u8]) -> (usize, usize) {
    let total = session_total();
    let mut start = cursor.max(session_first_cursor());
    if start >= total || out.is_empty() {
        return (0, total);
    }
    let index = start % SHELL_LOG_SIZE;
    let count = (SHELL_LOG_SIZE - index).min(total - start).min(out.len());
    unsafe {
        out[..count].copy_from_slice(&SHELL_BUF[index..index + count]);
    }
    start += count;
    (count, start)
}

/// Весь журнал shell как вектор.
pub fn session_bytes() -> Vec<u8> {
    let mut data = Vec::with_capacity(SHELL_LOG_SIZE);
    let mut cursor = session_first_cursor();
    loop {
        let total = session_total();
        if cursor >= total {
            break;
        }
        let index = cursor % SHELL_LOG_SIZE;
        let count = (SHELL_LOG_SIZE - index).min(total - cursor);
        unsafe {
            data.extend_from_slice(&SHELL_BUF[index..index + count]);
        }
        cursor += count;
    }
    data
}

// ==========================================
// Приёмники
// ==========================================

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

/// Интервал между записями лога на USB-носитель.
///
/// Носитель может быть медленным (флешка отвечает сотнями миллисекунд на
/// команду), а `checkpoint()` вызывается часто — после каждой команды shell
/// и раз в секунду из простоя. Без троттлинга каждая команда ждала запись
/// лога, и ввод выглядел «залипшим».
static NEXT_USB_FLUSH_MS: AtomicUsize = AtomicUsize::new(0);
static USB_FLUSH_INTERVAL_MS: AtomicUsize = AtomicUsize::new(1_000);

/// Сбросить накопленный лог на носители (checkpoint).
///
/// Вызывается на ключевых шагах boot, раз в секунду из idle-цикла shell
/// и после каждой команды. Запись на USB троттлится: если она оказалась
/// медленной, следующий сброс откладывается, а не тормозит вызвавшего.
pub fn checkpoint() {
    checkpoint_raw_disk();
    usb_flush_throttled();
    refresh_ramfs_logs(false);
}

/// Запись на USB с самонастройкой интервала.
fn usb_flush_throttled() {
    let now = crate::timer::uptime_ms() as usize;
    if now < NEXT_USB_FLUSH_MS.load(Ordering::Relaxed) {
        return;
    }

    let started = crate::timer::uptime_ms();
    usb::flush(false);
    let took = crate::timer::uptime_ms().saturating_sub(started) as usize;

    let mut interval = USB_FLUSH_INTERVAL_MS.load(Ordering::Relaxed);
    if took > 100 {
        // Медленный носитель: отступаем, вместо того чтобы копить задержку
        // на каждом вызове. Верхняя граница — 15 с, чтобы лог всё же шёл.
        interval = (interval * 2).min(15_000);
        USB_FLUSH_INTERVAL_MS.store(interval, Ordering::Relaxed);
    } else if took < 20 && interval > 1_000 {
        // Носитель снова быстрый — возвращаемся к обычному темпу.
        USB_FLUSH_INTERVAL_MS.store(1_000, Ordering::Relaxed);
        interval = 1_000;
    }
    NEXT_USB_FLUSH_MS.store(now.saturating_add(interval), Ordering::Relaxed);
}

/// Немедленный сброс лога без троттлинга.
///
/// Нужен там, где данные обязательно должны попасть на носитель до
/// выключения: `quit`, `reboot`, `sync`.
pub fn checkpoint_now() {
    checkpoint_raw_disk();
    usb::flush(false);
    refresh_ramfs_logs(false);
    NEXT_USB_FLUSH_MS.store(0, Ordering::Relaxed);
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

/// Сохранить журнал shell в файл ramfs.
pub fn flush_session_to_file(path: &str) {
    let data = session_bytes();
    if data.is_empty() {
        return;
    }
    let _ = crate::fs::write_file(path, &data);
}

/// Обновить файлы в ramfs, если лог заметно вырос.
///
/// Перезапись 256 КиБ на каждую секунду простоя бессмысленна, поэтому
/// ramfs-копии обновляются порциями; диск получает данные инкрементально
/// через `usb::flush` и всегда актуален.
fn refresh_ramfs_logs(force: bool) {
    const STEP: usize = 16 * 1024;

    let boot_total = total_len();
    let boot_written = RAMFS_BOOT_WRITTEN.load(Ordering::Relaxed);
    if force || boot_total.saturating_sub(boot_written) >= STEP {
        flush_to_file("/var/log/boot.log");
        RAMFS_BOOT_WRITTEN.store(boot_total, Ordering::Relaxed);
    }

    if !session_active() {
        return;
    }
    let shell_total = session_total();
    let shell_written = RAMFS_SHELL_WRITTEN.load(Ordering::Relaxed);
    if force || shell_total.saturating_sub(shell_written) >= STEP {
        flush_session_to_file("/var/log/shell.log");
        RAMFS_SHELL_WRITTEN.store(shell_total, Ordering::Relaxed);
    }
}

/// Немедленно записать оба журнала в ramfs (перед выключением, `sync`).
pub fn flush_all_files() {
    refresh_ramfs_logs(true);
}

/// Дописать журналы в ramfs, если накопилось достаточно нового.
///
/// Используется shell-циклом после каждой команды: `flush_all_files`
/// перезаписывал до 256 КиБ boot-лога и 64 КиБ shell-лога на каждую
/// команду, что на реальном железе добавляло заметную задержку.
pub fn refresh_files() {
    // Журнал shell пишется целиком после каждой команды: он небольшой
    // (64 КиБ кольцо), а порог в 16 КиБ означал, что `cat` показывал
    // устаревшее содержимое.
    if session_active() {
        flush_session_to_file("/var/log/shell.log");
        RAMFS_SHELL_WRITTEN.store(session_total(), Ordering::Relaxed);
    }
    refresh_ramfs_logs(false);
}

/// Инициализация: raw-запись + файлы в ramfs + сводка в serial.
///
/// Запись на USB запускается раньше, из `boot_log::usb::attach()` —
/// сразу после того, как найден накопитель.
pub fn init() {
    flush_raw();

    flush_to_file("/var/log/boot.log");
    RAMFS_BOOT_WRITTEN.store(total_len(), Ordering::Relaxed);

    let len = total_len();
    println!("[LOG] Boot log saved ({} bytes)", len);
    println!("[LOG] /var/log/boot.log");

    // Полный лог уже транслировался в serial/framebuffer построчно. Повторная
    // печать всех 256 КиБ на реальном UART занимает десятки секунд, поэтому
    // она доступна только по запросу: `log` или `log <N>` в shell.
}
