use core::fmt::Write;

const COM1: u16 = 0x3F8;
const LINE_STATUS: u16 = COM1 + 5;
const TRANSMITTER_EMPTY: u8 = 0x20;
const DATA_READY: u8 = 0x01;

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // выключить UART interrupts: читаем polling-ом
        outb(COM1 + 3, 0x80); // DLAB
        outb(COM1, 0x01);     // divisor low: 115200 baud
        outb(COM1 + 1, 0x00); // divisor high
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO enabled, clear, 14-byte threshold
        outb(COM1 + 4, 0x0B); // DTR + RTS + OUT2
    }
}

pub fn write_byte(b: u8) {
    // Записываем в boot log буфер (всегда, даже до ФС)
    crate::boot_log::log_byte(b);

    unsafe {
        while (inb(LINE_STATUS) & TRANSMITTER_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        outb(COM1, b);
    }
}

/// Неблокирующее чтение из UART. Безопасно вызывается из главного цикла:
/// проверяем DATA_READY до чтения DATA, поэтому порт никогда не ждёт байт.
#[cfg(feature = "serial-debug")]
fn try_read_byte() -> Option<u8> {
    unsafe {
        if inb(LINE_STATUS) & DATA_READY != 0 {
            Some(inb(COM1))
        } else {
            None
        }
    }
}

/// Преобразует байт serial terminal в тот же Key, что использует PS/2 driver.
/// Стрелки и escape sequences пока сознательно не разбираются: базовый shell
/// получает ASCII, Enter, Backspace и Tab. PS/2 остаётся полным источником
/// навигационных клавиш.
#[cfg(feature = "serial-debug")]
pub fn poll_key() -> Option<crate::keyboard::Key> {
    use crate::keyboard::Key;

    match try_read_byte()? {
        b'\r' | b'\n' => Some(Key::Enter),
        0x08 | 0x7f => Some(Key::Backspace),
        b'\t' => Some(Key::Tab),
        byte if (0x20..=0x7e).contains(&byte) => Some(Key::Char(byte as char)),
        _ => None,
    }
}

/// В production-сборке serial shell скомпилирован полностью вне ядра.
#[cfg(not(feature = "serial-debug"))]
pub fn poll_key() -> Option<crate::keyboard::Key> {
    None
}

/// Дублирует текст framebuffer-консоли в COM1 только в debug-сборке.
/// Это делает результат shell-команд доступным без окна QEMU.
#[cfg(feature = "serial-debug")]
pub fn mirror_console_char(ch: char) {
    if ch.is_ascii() {
        let byte = ch as u8;
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

#[cfg(not(feature = "serial-debug"))]
pub fn mirror_console_char(_ch: char) {}

pub fn write_str(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}

pub struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write_str(s);
        Ok(())
    }
}

pub fn _print(args: core::fmt::Arguments) {
    SerialWriter.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::serial::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
