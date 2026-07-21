// Boot Log — буфер для записи лога загрузки.
//
// Все println! попадают в этот буфер. При готовности ФС
// лог записывается в файл /var/log/boot.log для отладки
// на реальном железе.

use crate::println;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

const LOG_SIZE: usize = 32 * 1024; // 32 KB буфер

static mut LOG_BUF: [u8; LOG_SIZE] = [0; LOG_SIZE];
static LOG_POS: AtomicUsize = AtomicUsize::new(0);
static LOG_READY: AtomicBool = AtomicBool::new(false);

/// Записать байт в лог-буфер
pub fn log_byte(b: u8) {
    let pos = LOG_POS.fetch_add(1, Ordering::Relaxed);
    if pos < LOG_SIZE {
        unsafe { LOG_BUF[pos] = b; }
    }
}

/// Записать строку в лог-буфер
pub fn log_str(s: &str) {
    for b in s.bytes() {
        log_byte(b);
    }
}

/// Получить содержимое лога
pub fn get_log() -> &'static [u8] {
    let len = LOG_POS.load(Ordering::Relaxed);
    let actual_len = if len > LOG_SIZE { LOG_SIZE } else { len };
    unsafe { &LOG_BUF[..actual_len] }
}

/// Сохранить лог в файл (вызывается когда ФС готова)
pub fn flush_to_file(path: &str) {
    let log = get_log();
    if log.is_empty() { return; }

    use crate::fs;
    let _ = fs::remove_entry(path);
    if let Ok(_) = fs::create_file(path) {
        let _ = fs::write_file(path, log);
    }

    LOG_READY.store(true, Ordering::Relaxed);
}

/// Вызывается из main после init_filesystem()
pub fn init() {
    flush_to_file("/var/log/boot.log");
    flush_to_file("/var/log/kernel.log");
    println!("[LOG] Boot log saved to /var/log/boot.log ({} bytes)", LOG_POS.load(Ordering::Relaxed));
}
