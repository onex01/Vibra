use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use alloc::vec::Vec;
use spin::Mutex;

const COM1: u16 = 0x3F8;
const LINE_STATUS: u16 = COM1 + 5;
const TRANSMITTER_EMPTY: u8 = 0x20;
const DATA_READY: u8 = 0x01;
const RX_DATA: u16 = COM1; // Receive buffer register

/// Ring buffer для serial input (IRQ4 driven)
const SERIAL_BUF_SIZE: usize = 256;
static SERIAL_BUF: Mutex<[u8; SERIAL_BUF_SIZE]> = Mutex::new([0u8; SERIAL_BUF_SIZE]);
static SERIAL_HEAD: AtomicU16 = AtomicU16::new(0);
static SERIAL_TAIL: AtomicU16 = AtomicU16::new(0);

use core::sync::atomic::AtomicU16;

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
        // Выключаем interrupts на время конфигурации
        outb(COM1 + 1, 0x00);
        // DLAB + divisor
        outb(COM1 + 3, 0x80);
        outb(COM1, 0x01);     // 115200 baud low
        outb(COM1 + 1, 0x00); // high
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO enabled, clear, 14-byte threshold
        outb(COM1 + 4, 0x0B); // DTR + RTS + OUT2
        // Включаем receive interrupt (bit 0 = RX data ready)
        outb(COM1 + 1, 0x01);
    }
}

/// Вызывается из IRQ4 handler (isr_serial)
pub fn handle_interrupt() {
    unsafe {
        while inb(LINE_STATUS) & DATA_READY != 0 {
            let byte = inb(RX_DATA);
            push_byte(byte);
        }
    }
    // EOI
    if crate::interrupts::apic::is_active() {
        crate::interrupts::apic::eoi();
    } else {
        unsafe { crate::interrupts::pic::eoi(4); }
    }
}

fn push_byte(b: u8) {
    let head = SERIAL_HEAD.load(Ordering::Relaxed);
    let tail = SERIAL_TAIL.load(Ordering::Relaxed);
    let next = (head + 1) % SERIAL_BUF_SIZE as u16;
    if next != tail {
        let mut buf = SERIAL_BUF.lock();
        buf[head as usize] = b;
        drop(buf);
        SERIAL_HEAD.store(next, Ordering::Release);
    }
    // Если буфер полон — теряем байт (не блокируем ISR)
}

/// Pop байт из ring buffer (безопасно для poll_key)
fn pop_byte() -> Option<u8> {
    let head = SERIAL_HEAD.load(Ordering::Acquire);
    let tail = SERIAL_TAIL.load(Ordering::Relaxed);
    if head == tail {
        return None;
    }
    let buf = SERIAL_BUF.lock();
    let b = buf[tail as usize];
    drop(buf);
    SERIAL_TAIL.store((tail + 1) % SERIAL_BUF_SIZE as u16, Ordering::Release);
    Some(b)
}

pub fn write_byte(b: u8) {
    crate::boot_log::log_byte(b);
    unsafe {
        while (inb(LINE_STATUS) & TRANSMITTER_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        outb(COM1, b);
    }
}

#[cfg(feature = "serial-debug")]
fn try_read_byte() -> Option<u8> {
    // Сначала проверяем ring buffer (IRQ4 driven)
    if let Some(b) = pop_byte() {
        return Some(b);
    }
    // Fallback: polling (если IRQ4 не работает)
    unsafe {
        if inb(LINE_STATUS) & DATA_READY != 0 {
            Some(inb(RX_DATA))
        } else {
            None
        }
    }
}

#[cfg(feature = "serial-debug")]
pub fn poll_key() -> Option<crate::keyboard::Key> {
    use crate::keyboard::Key;

    match try_read_byte()? {
        b'\r' | b'\n' => Some(Key::Enter),
        0x08 | 0x7f => Some(Key::Backspace),
        b'\t' => Some(Key::Tab),
        // Ctrl+A — начало строки
        0x01 => Some(Key::Char('\x01')),
        // Ctrl+E — конец строки
        0x05 => Some(Key::Char('\x05')),
        // Ctrl+K — удалить до конца строки
        0x0B => Some(Key::Char('\x0B')),
        // Ctrl+L — очистить экран
        0x0C => Some(Key::Char('\x0C')),
        // Ctrl+U — удалить до начала строки
        0x15 => Some(Key::Char('\x15')),
        // Ctrl+Z
        0x1A => Some(Key::Char('\x1A')),
        // ESC sequence start (ansi arrow keys etc.)
        0x1B => Some(Key::Char('\x1B')),
        byte if (0x20..=0x7e).contains(&byte) => Some(Key::Char(byte as char)),
        _ => None,
    }
}

#[cfg(not(feature = "serial-debug"))]
pub fn poll_key() -> Option<crate::keyboard::Key> {
    None
}

/// Дублирует текст framebuffer-консоли в COM1.
pub fn mirror_console_char(ch: char) {
    #[cfg(feature = "serial-debug")]
    {
        if ch.is_ascii() {
            let byte = ch as u8;
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
    }
}

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
