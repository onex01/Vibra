use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "serial-debug")]
use core::sync::atomic::{AtomicU16, AtomicU8};
#[cfg(feature = "serial-debug")]
use spin::Mutex;

// Всё, что связано с UART, компилируется только в debug-сборке
// (`serial-debug` включён по умолчанию, снимается `--no-default-features`).
// В release COM1 не инициализируется и не опрашивается: канала ввода и
// вывода через UART в бинаре не остаётся. Вывод boot-лога на экран от этого
// не зависит — `write_byte` отдельно зовёт `framebuffer::write_boot_byte`.
#[cfg(feature = "serial-debug")]
const COM1: u16 = 0x3F8;
#[cfg(feature = "serial-debug")]
const LINE_STATUS: u16 = COM1 + 5;
#[cfg(feature = "serial-debug")]
const TRANSMITTER_EMPTY: u8 = 0x20;
#[cfg(feature = "serial-debug")]
const DATA_READY: u8 = 0x01;
#[cfg(feature = "serial-debug")]
const RX_DATA: u16 = COM1; // Receive buffer register
#[cfg(feature = "serial-debug")]
const MODEM_CONTROL: u16 = COM1 + 4;
#[cfg(feature = "serial-debug")]
const SCRATCH: u16 = COM1 + 7;

// Сколько итераций ждать THRE, прежде чем признать UART мёртвым.
// Без ограничения отсутствующий/сломанный COM1 вешает ядро навсегда
// (ровно на первом println! после смены CR3 на реальном железе).
#[cfg(feature = "serial-debug")]
const TX_SPIN_LIMIT: u32 = 100_000;

/// Отвечает ли COM1 на loopback-пробу. На ноутбуках без физического UART
/// порт может читаться как 0x00 (THRE всегда 0) — тогда писать в него нельзя.
static SERIAL_PRESENT: AtomicBool = AtomicBool::new(false);

/// Ring buffer для serial input (IRQ4 driven)
#[cfg(feature = "serial-debug")]
const SERIAL_BUF_SIZE: usize = 256;
#[cfg(feature = "serial-debug")]
static SERIAL_BUF: Mutex<[u8; SERIAL_BUF_SIZE]> = Mutex::new([0u8; SERIAL_BUF_SIZE]);
#[cfg(feature = "serial-debug")]
static SERIAL_HEAD: AtomicU16 = AtomicU16::new(0);
#[cfg(feature = "serial-debug")]
static SERIAL_TAIL: AtomicU16 = AtomicU16::new(0);
#[cfg(feature = "serial-debug")]
static SERIAL_ESCAPE_STATE: AtomicU8 = AtomicU8::new(0);

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
#[cfg(feature = "serial-debug")]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

/// Инициализация COM1 (только debug-сборка).
///
/// В release UART не трогаем вообще: ни loopback-пробы, ни записи в порты
/// 0x3F8..0x3FF. `is_present()` тогда всегда `false`, а `write_serial_byte`
/// ничего не делает.
#[cfg(feature = "serial-debug")]
pub fn init() {
    unsafe {
        // Проба UART: loopback + scratch-регистр (0x3F8+7).
        // Только после успешной пробы можно доверять LSR.THRE.
        outb(MODEM_CONTROL, 0x1E); // loopback: DTR|RTS|OUT1|OUT2
        outb(SCRATCH, 0xAE);
        let present = inb(SCRATCH) == 0xAE;
        outb(MODEM_CONTROL, 0x0F); // выходим из loopback
        SERIAL_PRESENT.store(present, Ordering::SeqCst);

        if !present {
            return;
        }

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

/// Release-сборка: UART выключен, порты не инициализируются.
#[cfg(not(feature = "serial-debug"))]
pub fn init() {
    SERIAL_PRESENT.store(false, Ordering::SeqCst);
}

/// Есть ли на машине живой COM1 (loopback-проба в `init`).
pub fn is_present() -> bool {
    SERIAL_PRESENT.load(Ordering::Relaxed)
}

/// Вызывается из IRQ4 handler (isr_serial).
///
/// Только debug: в release IRQ4 замаскирован, а сам обработчик в IDT не
/// регистрируется (см. `interrupts/idt.rs` и `interrupts/pic.rs`).
#[cfg(feature = "serial-debug")]
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

#[cfg(feature = "serial-debug")]
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
#[cfg(feature = "serial-debug")]
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

#[cfg(feature = "serial-debug")]
fn write_byte_without_framebuffer(b: u8) {
    crate::boot_log::log_byte(b);
    write_serial_byte(b);
}

#[cfg(feature = "serial-debug")]
pub(crate) fn write_serial_byte(b: u8) {
    // Без живого UART не ждём THRE вообще: на части машин порт читается как
    // 0x00 и неограниченный спин превращается в вечное зависание ядра.
    if !SERIAL_PRESENT.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let mut spins = 0u32;
        while (inb(LINE_STATUS) & TRANSMITTER_EMPTY) == 0 {
            core::hint::spin_loop();
            spins += 1;
            if spins >= TX_SPIN_LIMIT {
                return;
            }
        }
        outb(COM1, b);
    }
}

/// Release-сборка: байт никуда не уходит.
///
/// Функция пустая и `#[inline(always)]`, поэтому из бинаря исчезают и спин
/// на THRE, и сам `out` в 0x3F8 — остаётся только `boot_log` и framebuffer.
#[cfg(not(feature = "serial-debug"))]
#[inline(always)]
pub(crate) fn write_serial_byte(_b: u8) {}

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
        // Ctrl+C — отмена текущей команды/строки
        0x03 => Some(Key::Char('\x03')),
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
///
/// В release-сборке зеркало отключено: экран и `boot_log` пишутся отдельно,
/// поэтому вывод не теряется, а байт в UART не уходит.
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
    #[cfg(not(feature = "serial-debug"))]
    let _ = ch;
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
