// PS/2 keyboard driver (i8042 / 8042-compatible controller).
//
// Почему драйвер выглядит именно так:
//   * На ноутбуках клавиатура почти всегда висит на i8042 (EC), а не на USB.
//     Прошивка может оставить scanning выключенным, IRQ1 замаскированным или
//     перенаправленным на IOAPIC — поэтому одного IRQ-пути недостаточно.
//   * `poll_key()` дополнительно опрашивает порт 0x64 (polling fallback):
//     ввод продолжает работать, даже если прерывания не доходят.
//   * Контроллер может не принять бит translation (config byte bit 6). Тогда
//     клавиатура отдаёт Scan Code Set 2 — драйвер декодирует и его.
//   * Никакого `println!` из ISR: serial/framebuffer используют spin-локи,
//     и печать из прерывания вешает машину, если лок уже взят основным кодом.

use crate::println;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Unknown,
}

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;
const PS2_COMMAND: u16 = 0x64;

/// Ответы/служебные байты клавиатуры, которые не являются сканкодами.
const KBD_ACK: u8 = 0xFA;
const KBD_RESEND: u8 = 0xFE;
const KBD_ECHO: u8 = 0xEE;
const KBD_SELF_TEST_OK: u8 = 0xAA;
const KBD_OVERRUN: u8 = 0xFF;
const KBD_ERROR: u8 = 0x00;

const QUEUE_LEN: usize = 64;
const IO_SPIN_LIMIT: u32 = 200_000;

struct KeyboardState {
    queue: [u8; QUEUE_LEN],
    head: usize,
    tail: usize,

    shift: bool,
    ctrl: bool,
    alt: bool,
    caps: bool,

    /// Префикс E0 (расширенная клавиша).
    extended: bool,
    /// Префикс F0 (release в Scan Code Set 2).
    release: bool,
    /// true — контроллер отдаёт Scan Code Set 2 без трансляции в Set 1.
    scan_set2: bool,

    present: bool,
    config: u8,
    irq_count: u64,
    dropped: u64,
}

impl KeyboardState {
    const fn new() -> Self {
        KeyboardState {
            queue: [0; QUEUE_LEN],
            head: 0,
            tail: 0,
            shift: false,
            ctrl: false,
            alt: false,
            caps: false,
            extended: false,
            release: false,
            scan_set2: false,
            present: false,
            config: 0,
            irq_count: 0,
            dropped: 0,
        }
    }

    fn push_scancode(&mut self, scancode: u8) {
        let next = (self.head + 1) % QUEUE_LEN;
        if next != self.tail {
            self.queue[self.head] = scancode;
            self.head = next;
        } else {
            // Переполнение очереди: потерянный байт вполне может быть break-кодом
            // модификатора. Если его не учесть, Shift останется «зажатым» и весь
            // дальнейший ввод уйдёт в верхний регистр — сбрасываем модификаторы.
            self.dropped = self.dropped.wrapping_add(1);
            self.shift = false;
            self.ctrl = false;
            self.alt = false;
        }
    }

    fn pop_scancode(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let sc = self.queue[self.tail];
            self.tail = (self.tail + 1) % QUEUE_LEN;
            Some(sc)
        }
    }

    fn queued(&self) -> usize {
        if self.head >= self.tail {
            self.head - self.tail
        } else {
            QUEUE_LEN - self.tail + self.head
        }
    }

    /// Достаёт следующий осмысленный `Key`, пропуская служебные байты.
    fn next_key(&mut self) -> Option<Key> {
        loop {
            let sc = self.pop_scancode()?;
            match sc {
                KBD_ACK | KBD_RESEND | KBD_ECHO | KBD_SELF_TEST_OK | KBD_OVERRUN | KBD_ERROR => {
                    continue;
                }
                _ => {}
            }

            let decoded = if self.scan_set2 {
                self.decode_set2(sc)
            } else {
                self.decode_set1(sc)
            };
            if let Some(key) = decoded {
                return Some(key);
            }
        }
    }

    fn decode_set1(&mut self, sc: u8) -> Option<Key> {
        if sc == 0xE0 {
            self.extended = true;
            return None;
        }

        let released = sc & 0x80 != 0;
        let code = sc & 0x7F;

        if self.extended {
            self.extended = false;
            if released {
                return None;
            }
            return match code {
                0x4B => Some(Key::Left),
                0x4D => Some(Key::Right),
                0x48 => Some(Key::Up),
                0x50 => Some(Key::Down),
                0x1C => Some(Key::Enter), // keypad Enter
                _ => None,
            };
        }

        match code {
            0x2A | 0x36 => {
                self.shift = !released;
                return None;
            }
            0x1D => {
                self.ctrl = !released;
                return None;
            }
            0x38 => {
                self.alt = !released;
                return None;
            }
            0x3A => {
                if !released {
                    self.caps = !self.caps;
                }
                return None;
            }
            _ => {}
        }

        if released {
            return None;
        }

        let ch = set1_char(code)?;
        Some(self.translate(ch))
    }

    fn decode_set2(&mut self, sc: u8) -> Option<Key> {
        if sc == 0xE0 {
            self.extended = true;
            return None;
        }
        if sc == 0xF0 {
            self.release = true;
            return None;
        }

        let released = self.release;
        self.release = false;

        if self.extended {
            self.extended = false;
            if released {
                return None;
            }
            return match sc {
                0x6B => Some(Key::Left),
                0x74 => Some(Key::Right),
                0x75 => Some(Key::Up),
                0x72 => Some(Key::Down),
                0x5A => Some(Key::Enter), // keypad Enter
                _ => None,
            };
        }

        match sc {
            0x12 | 0x59 => {
                self.shift = !released;
                return None;
            }
            0x14 => {
                self.ctrl = !released;
                return None;
            }
            0x11 => {
                self.alt = !released;
                return None;
            }
            0x58 => {
                if !released {
                    self.caps = !self.caps;
                }
                return None;
            }
            _ => {}
        }

        if released {
            return None;
        }

        let ch = set2_char(sc)?;
        Some(self.translate(ch))
    }

    fn translate(&self, ch: char) -> Key {
        // Ctrl+<letter> → управляющий код (Ctrl+C = 0x03, Ctrl+Z = 0x1A).
        // Ctrl+C сразу поднимает флаг отмены: длинные команды и скрипты
        // проверяют его в своих циклах.
        if self.ctrl && ch.is_ascii_alphabetic() {
            let ctrl = ((ch.to_ascii_lowercase() as u8) - b'a' + 1) as char;
            if ctrl == '\x03' {
                crate::request_cancel();
            }
            return Key::Char(ctrl);
        }

        let ch = if ch.is_ascii_alphabetic() {
            if self.caps ^ self.shift {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            }
        } else if self.shift {
            shift_symbol(ch)
        } else {
            ch
        };

        match ch {
            '\n' => Key::Enter,
            '\x08' => Key::Backspace,
            '\t' => Key::Tab,
            _ => Key::Char(ch),
        }
    }
}

/// US-раскладка: символ при нажатом Shift.
pub(crate) fn shift_symbol(ch: char) -> char {
    match ch {
        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
        '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
        '-' => '_', '=' => '+', '[' => '{', ']' => '}', '\\' => '|',
        ';' => ':', '\'' => '"', '`' => '~', ',' => '<', '.' => '>',
        '/' => '?',
        other => other,
    }
}

/// Scan Code Set 1 (после трансляции контроллером), make-код.
fn set1_char(code: u8) -> Option<char> {
    Some(match code {
        0x01 => '\x1B',
        0x02 => '1', 0x03 => '2', 0x04 => '3', 0x05 => '4', 0x06 => '5',
        0x07 => '6', 0x08 => '7', 0x09 => '8', 0x0A => '9', 0x0B => '0',
        0x0C => '-', 0x0D => '=',
        0x0E => '\x08',
        0x0F => '\t',
        0x10 => 'q', 0x11 => 'w', 0x12 => 'e', 0x13 => 'r', 0x14 => 't',
        0x15 => 'y', 0x16 => 'u', 0x17 => 'i', 0x18 => 'o', 0x19 => 'p',
        0x1A => '[', 0x1B => ']',
        0x1C => '\n',
        0x1E => 'a', 0x1F => 's', 0x20 => 'd', 0x21 => 'f', 0x22 => 'g',
        0x23 => 'h', 0x24 => 'j', 0x25 => 'k', 0x26 => 'l', 0x27 => ';',
        0x28 => '\'', 0x29 => '`',
        0x2B => '\\',
        0x2C => 'z', 0x2D => 'x', 0x2E => 'c', 0x2F => 'v', 0x30 => 'b',
        0x31 => 'n', 0x32 => 'm', 0x33 => ',', 0x34 => '.', 0x35 => '/',
        0x39 => ' ',
        _ => return None,
    })
}

/// Scan Code Set 2, make-код (без префиксов E0/F0).
fn set2_char(code: u8) -> Option<char> {
    Some(match code {
        0x76 => '\x1B',
        0x16 => '1', 0x1E => '2', 0x26 => '3', 0x25 => '4', 0x2E => '5',
        0x36 => '6', 0x3D => '7', 0x3E => '8', 0x46 => '9', 0x45 => '0',
        0x4E => '-', 0x55 => '=',
        0x66 => '\x08',
        0x0D => '\t',
        0x15 => 'q', 0x1D => 'w', 0x24 => 'e', 0x2D => 'r', 0x2C => 't',
        0x35 => 'y', 0x3C => 'u', 0x43 => 'i', 0x44 => 'o', 0x4D => 'p',
        0x54 => '[', 0x5B => ']',
        0x5A => '\n',
        0x1C => 'a', 0x1B => 's', 0x23 => 'd', 0x2B => 'f', 0x34 => 'g',
        0x33 => 'h', 0x3B => 'j', 0x42 => 'k', 0x4B => 'l', 0x4C => ';',
        0x52 => '\'', 0x0E => '`',
        0x5D => '\\',
        0x1A => 'z', 0x22 => 'x', 0x21 => 'c', 0x2A => 'v', 0x32 => 'b',
        0x31 => 'n', 0x3A => 'm', 0x41 => ',', 0x49 => '.', 0x4A => '/',
        0x29 => ' ',
        _ => return None,
    })
}

static KEYBOARD_STATE: Mutex<KeyboardState> = Mutex::new(KeyboardState::new());

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
    val
}

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}

#[inline]
unsafe fn io_wait() {
    outb(0x80, 0);
}

/// Ждём, пока входной буфер контроллера опустеет (status bit 1 == 0).
unsafe fn wait_input_empty() -> bool {
    for _ in 0..IO_SPIN_LIMIT {
        if inb(PS2_STATUS) & 0x02 == 0 {
            return true;
        }
    }
    false
}

/// Ждём данные от контроллера (status bit 0 == 1).
unsafe fn wait_output_full() -> bool {
    for _ in 0..IO_SPIN_LIMIT {
        if inb(PS2_STATUS) & 0x01 != 0 {
            return true;
        }
    }
    false
}

unsafe fn write_command(cmd: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    outb(PS2_COMMAND, cmd);
    true
}

unsafe fn write_data(data: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    outb(PS2_DATA, data);
    true
}

unsafe fn read_data() -> Option<u8> {
    if wait_output_full() {
        Some(inb(PS2_DATA))
    } else {
        None
    }
}

/// Читает и отбрасывает всё, что уже лежит в выходном буфере.
unsafe fn flush() -> usize {
    let mut count = 0;
    for _ in 0..64 {
        if inb(PS2_STATUS) & 0x01 == 0 {
            break;
        }
        inb(PS2_DATA);
        count += 1;
    }
    count
}

/// Команда самой клавиатуре (через порт данных) с ожиданием ACK.
unsafe fn keyboard_command(cmd: u8) -> bool {
    for _ in 0..3 {
        if !write_data(cmd) {
            return false;
        }
        for _ in 0..IO_SPIN_LIMIT {
            if inb(PS2_STATUS) & 0x01 == 0 {
                continue;
            }
            match inb(PS2_DATA) {
                KBD_ACK => return true,
                KBD_RESEND => break, // отправим ещё раз
                _ => {}              // эхо/мусор — игнорируем
            }
        }
    }
    false
}

/// Есть ли вообще контроллер: 0xFF на обоих портах означает «нет устройства».
fn controller_present() -> bool {
    unsafe {
        let status = inb(PS2_STATUS);
        let data = inb(PS2_DATA);
        status != 0xFF || data != 0xFF
    }
}

pub fn init() {
    if !controller_present() {
        println!("[KEYBOARD] no i8042 controller (ports 0x60/0x64 read 0xff) — PS/2 disabled");
        return;
    }

    if !crate::acpi::has_8042() {
        println!("[KEYBOARD] ACPI FADT reports no 8042; probing the controller anyway");
    }

    unsafe {
        // Чистое состояние: выключаем оба порта, затем сбрасываем контроллер.
        flush();
        write_command(0xAD); // disable keyboard port
        write_command(0xA7); // disable aux (mouse) port
        flush();

        if write_command(0xAA) {
            // self-test; на части ноутбучных EC команда не поддерживается
            if let Some(result) = read_data() {
                if result == 0x55 {
                    println!("[KEYBOARD] i8042 self-test OK");
                } else {
                    println!("[KEYBOARD] i8042 self-test returned {:#x} (continuing)", result);
                }
            }
        }
        flush();

        // Configuration byte: IRQ1, system flag, translation Set2 -> Set1,
        // часы клавиатуры и мыши не заблокированы.
        let mut config = 0u8;
        if write_command(0x20) {
            if let Some(value) = read_data() {
                config = value;
            }
        }
        let previous = config;
        config |= 0x01; // IRQ1 enable
        config |= 0x04; // system flag
        config |= 0x40; // translation to Scan Code Set 1
        config &= !0x08; // ignore-keyboard-clock = 0
        config &= !0x10; // keyboard clock enabled
        config &= !0x20; // aux clock enabled (PS/2 mouse продолжит работать)

        let written = write_command(0x60) && write_data(config);
        let mut applied = config;
        if written {
            // Проверяем, что контроллер принял translation: иначе Set 2.
            if write_command(0x20) {
                if let Some(value) = read_data() {
                    applied = value;
                }
            }
        }

        // Включаем порт клавиатуры и scanning.
        write_command(0xAE);
        io_wait();
        flush();
        let scanning = keyboard_command(0xF4);
        flush();

        let mut state = KEYBOARD_STATE.lock();
        state.present = true;
        state.config = applied;
        state.scan_set2 = applied & 0x40 == 0;

        println!(
            "[KEYBOARD] i8042 ready: cfg {:#x} -> {:#x}, scan set {}, scanning {}",
            previous,
            applied,
            if state.scan_set2 { 2 } else { 1 },
            if scanning { "enabled" } else { "NOT enabled" }
        );
        if state.scan_set2 {
            println!("[KEYBOARD] translation bit not applied — decoding Scan Code Set 2");
        }
    }
}

/// Повторная настройка после PIC remap / IDT setup.
/// Вызывается из `init()` уже с включёнными прерываниями.
pub fn post_init() {
    let present = KEYBOARD_STATE.lock().present;
    if !present {
        return;
    }

    unsafe {
        // На случай, если прошивка оставила IRQ1 замаскированным.
        crate::interrupts::pic::unmask_irq1();

        // Включаем scanning ещё раз: некоторые EC сбрасывают его при self-test.
        if !keyboard_command(0xF4) {
            write_command(0xAE);
            io_wait();
        }

        // Убираем накопившийся мусор, чтобы первая клавиша не «потерялась».
        let flushed = flush();
        let status = inb(PS2_STATUS);
        let state = KEYBOARD_STATE.lock();
        println!(
            "[KEYBOARD] IRQ1 unmasked, status={:#x}, scan set {}, flushed {} byte(s)",
            status,
            if state.scan_set2 { 2 } else { 1 },
            flushed
        );
    }
}

/// Вызывается из IRQ1. Читает данные контроллера и складывает их в очередь.
pub fn handle_interrupt() {
    // Вычитываем ВСЁ, что контроллер успел положить в буфер: IRQ1 — фронтовый,
    // и если прочитать один байт из двух, второй останется в буфере без нового
    // фронта. При следующем нажатии он будет перезаписан (overrun) — так теряются
    // break-коды модификаторов и «залипает» Shift, а также пропадает Enter.
    unsafe {
        for _ in 0..32 {
            let status = inb(PS2_STATUS);
            // bit 0 = data available, bit 5 = данные от AUX (мышь) — не наши.
            if status & 0x01 == 0 || status & 0x20 != 0 {
                break;
            }
            let scancode = inb(PS2_DATA);
            let mut state = KEYBOARD_STATE.lock();
            state.irq_count = state.irq_count.wrapping_add(1);
            state.push_scancode(scancode);
        }
    }
}

/// Polling fallback: забирает байты из контроллера без участия IRQ1.
fn drain_controller(state: &mut KeyboardState) -> usize {
    let mut count = 0;
    unsafe {
        for _ in 0..QUEUE_LEN {
            let status = inb(PS2_STATUS);
            if status == 0xFF || status & 0x01 == 0 {
                break;
            }
            let data = inb(PS2_DATA);
            if status & 0x20 == 0 {
                state.push_scancode(data);
            }
            count += 1;
        }
    }
    count
}

pub fn poll_key() -> Option<Key> {
    // USB HID (xHCI) опрашивается до отключения прерываний: MMIO-ожидания
    // не должны идти с выключенным IRQ.
    crate::drivers::usb::xhci::poll_keyboard();

    // Лок делится с ISR — берём его только с выключенными прерываниями,
    // иначе IRQ1 во время удержания лока приведёт к дедлоку.
    crate::interrupts::without_interrupts(|| {
        // Сначала проверяем очередь HID (usb) — она хранит уже распарсенные Key
        {
            let mut hid_q = HID_QUEUE.lock();
            if !hid_q.is_empty() {
                return Some(hid_q.remove(0));
            }
        }

        let mut state = KEYBOARD_STATE.lock();
        if !state.present {
            return None;
        }
        // Polling fallback: работает, даже если IRQ1 не доставляется.
        drain_controller(&mut state);
        state.next_key()
    })
}

use alloc::vec::Vec;
// Очередь для внешней (USB/HID) инъекции клавиш
static HID_QUEUE: Mutex<Vec<Key>> = Mutex::new(Vec::new());

/// Внедрить `Key` из внешнего источника (USB/HID). Безопасно вызывать из драйвера.
pub fn inject_key(k: Key) {
    let mut q = HID_QUEUE.lock();
    q.push(k);
}

/// Снимок состояния драйвера для `diag keyboard`.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardStatus {
    pub present: bool,
    pub config: u8,
    pub scan_set2: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub irq_count: u64,
    pub dropped: u64,
    pub queued: usize,
    pub hid_queued: usize,
}

pub fn status() -> KeyboardStatus {
    // Лок KEYBOARD_STATE делится с IRQ1: без cli возможен дедлок, если
    // прерывание придёт между lock() и drop().
    crate::interrupts::without_interrupts(|| {
        let state = KEYBOARD_STATE.lock();
        let hid_queued = HID_QUEUE.lock().len();
        KeyboardStatus {
            present: state.present,
            config: state.config,
            scan_set2: state.scan_set2,
            shift: state.shift,
            ctrl: state.ctrl,
            alt: state.alt,
            irq_count: state.irq_count,
            dropped: state.dropped,
            queued: state.queued(),
            hid_queued,
        }
    })
}

/// Debug: состояние очередей на момент завершения boot.
/// Намеренно не опрашивает железо: первый `poll_key()` из shell запускает
/// ленивое перечисление USB уже после старта таймера.
pub fn dump_pending() {
    let state = status();
    println!(
        "[KEYBOARD] IRQ count = {}, queued PS/2 = {}, USB HID = {}",
        state.irq_count, state.queued, state.hid_queued
    );
}

pub fn irq_count() -> u64 {
    KEYBOARD_STATE.lock().irq_count
}




