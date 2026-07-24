// Boot Log — буфер + raw запись на диск для отладки на реальном железе.
//
// Все println! попадают в ring buffer (32KB).
// Лог записывается:
//   1. Raw в сектор 2 FAT32 (первый свободный sector после boot sector)
//      — работает ДО инициализации ФС, данные сохраняются при падении ядра.
//   2. В файл /var/log/boot.log — после init_filesystem().
//
// На реальном железе: открыть USB-флешку, прочитать sector 2 в hex-редакторе.

use crate::println;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const LOG_SIZE: usize = 32 * 1024; // 32 KB буфер

static mut LOG_BUF: [u8; LOG_SIZE] = [0; LOG_SIZE];
static LOG_POS: AtomicUsize = AtomicUsize::new(0);
static LOG_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Записать байт в лог-буфер
pub fn log_byte(b: u8) {
    let pos = LOG_POS.fetch_add(1, Ordering::Relaxed);
    if pos < LOG_SIZE {
        unsafe { LOG_BUF[pos] = b; }
    }
}

/// Получить содержимое лога
pub fn get_log() -> &'static [u8] {
    let len = LOG_POS.load(Ordering::Relaxed);
    let actual_len = if len > LOG_SIZE { LOG_SIZE } else { len };
    unsafe { &LOG_BUF[..actual_len] }
}

/// Raw запись лога на диск (через AHCI) — работает ДО init_filesystem().
/// Записывает в сектор 2 (LBA=2) на первом SATA порту.
/// Сектор 2 — первый свободный sector после boot sector (LBA 0) и FSInfo (LBA 1).
pub fn flush_raw() {
    if LOG_WRITTEN.load(Ordering::Relaxed) { return; }

    let log = get_log();
    if log.is_empty() { return; }

    // Ищем первый активный AHCI порт
    let port = match crate::drivers::ahci::first_port() {
        Some(p) => p,
        None => return, // Нет SATA диска — пропускаем
    };

    // Записываем лог в сектор LBA=2 (размер сектора = 512 байт)
    let mut sector = [0u8; 512];
    let copy_len = log.len().min(512);
    sector[..copy_len].copy_from_slice(&log[..copy_len]);

    if crate::drivers::ahci::write_sectors(port, 2, 1, &sector) {
        LOG_WRITTEN.store(true, Ordering::Relaxed);
    }
}

/// Сохранить лог в файл (после init_filesystem)
pub fn flush_to_file(path: &str) {
    let log = get_log();
    if log.is_empty() { return; }

    use crate::fs;
    let _ = fs::remove_entry(path);
    if let Ok(_) = fs::create_file(path) {
        let _ = fs::write_file(path, log);
    }
}

/// Инициализация: raw запись + файл
pub fn init() {
    // Raw запись на диск (до ФС)
    flush_raw();

    // Файловая запись в /var/log/ (после ФС)
    flush_to_file("/var/log/boot.log");
    flush_to_file("/var/log/kernel.log");

    // Записываем boot.log в КОРЕНЬ раздела (рядом с kernel.elf)
    // Это позволяет прочитать лог на реальном железе с любой ОС
    flush_to_file("/boot.log");

    let len = LOG_POS.load(Ordering::Relaxed);
    println!("[LOG] Boot log saved ({} bytes)", len);
    println!("[LOG] /boot.log + /var/log/boot.log + sector 2 on SATA disk");

    // Дополнительно: выводим лог в serial (для отладки на компьютере с serial)
    if len > 0 {
        println!("[LOG] === FULL BOOT LOG ===");
        let log = get_log();
        // Печатаем лог построчно через serial
        for &b in log {
            if b == b'\n' || b == b'\r' {
                crate::serial::write_byte(b);
            } else if b >= 0x20 && b < 0x7F {
                crate::serial::write_byte(b);
            }
        }
        crate::serial::write_byte(b'\n');
        crate::serial::write_byte(b'\n');
    }
}
