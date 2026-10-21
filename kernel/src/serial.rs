use core::fmt::Write;
use core::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "serial-debug")]
use core::sync::atomic::AtomicU8;
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
#[cfg(feature = "serial-debug")]
static SERIAL_ESCAPE_STATE: AtomicU8 = AtomicU8::new(0);

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
    crate::interrupts::without_interrupts(|| {
        let head = SERIAL_HEAD.load(Ordering::Acquire);
        let tail = SERIAL_TAIL.load(Ordering::Relaxed);
        if head == tail {
            return None;
        }
        let buf = SERIAL_BUF.lock();
        let byte = buf[tail as usize];
        drop(buf);
        SERIAL_TAIL.store((tail + 1) % SERIAL_BUF_SIZE as u16, Ordering::Release);
        Some(byte)
    })
}

pub fn write_byte(b: u8) {
    crate::boot_log::log_byte(b);
    write_serial_byte(b);
    crate::framebuffer::write_boot_byte(b);
}

fn write_byte_without_framebuffer(b: u8) {
    crate::boot_log::log_byte(b);
    write_serial_byte(b);
}

pub(crate) fn write_serial_byte(b: u8) {
    unsafe {
        while (inb(LINE_STATUS) & TRANSMITTER_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        outb(COM1, b);
    }
}

struct DebugConWriter;

impl Write for DebugConWriter {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        unsafe {
            for byte in text.bytes() {
                outb(0xE9, byte);
            }
        }
        Ok(())
    }
}

pub fn debugcon_println(args: core::fmt::Arguments) {
    DebugConWriter.write_fmt(args).unwrap();
    unsafe { outb(0xE9, b'\n'); }
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
    loop {
        if let Some(key) = decode_serial_byte(try_read_byte()?) {
            return Some(key);
        }
    }
}

#[cfg(feature = "serial-debug")]
fn decode_serial_byte(byte: u8) -> Option<crate::keyboard::Key> {
    use crate::keyboard::Key;

    match SERIAL_ESCAPE_STATE.swap(0, Ordering::Relaxed) {
        1 => match byte {
            b'[' => {
                SERIAL_ESCAPE_STATE.store(2, Ordering::Relaxed);
                None
            }
            b'O' => {
                SERIAL_ESCAPE_STATE.store(3, Ordering::Relaxed);
                None
            }
            _ => decode_plain_byte(byte),
        },
        2 | 3 => match byte {
            b'A' => Some(Key::Up),
            b'B' => Some(Key::Down),
            b'C' => Some(Key::Right),
            b'D' => Some(Key::Left),
            b'H' => Some(Key::Char('\x01')),
            b'F' => Some(Key::Char('\x05')),
            _ => None,
        },
        _ if byte == 0x1B => {
            SERIAL_ESCAPE_STATE.store(1, Ordering::Relaxed);
            None
        }
        _ => decode_plain_byte(byte),
    }
}

#[cfg(feature = "serial-debug")]
fn decode_plain_byte(byte: u8) -> Option<crate::keyboard::Key> {
    use crate::keyboard::Key;

    match byte {
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
                write_byte_without_framebuffer(b'\r');
            }
            write_byte_without_framebuffer(byte);
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
