use crate::println;
use core::arch::asm;

const PS2_DATA_PORT: u16 = 0x60;
const PS2_STATUS_PORT: u16 = 0x64;

static mut SHIFT_PRESSED: bool = false;
static mut EXTENDED_KEY: bool = false;

const SCANCODE_LSHIFT: u8 = 0x2A;
const SCANCODE_RSHIFT: u8 = 0x36;
const SCANCODE_LSHIFT_RELEASE: u8 = 0xAA;
const SCANCODE_RSHIFT_RELEASE: u8 = 0xB6;
const SCANCODE_EXTENDED: u8 = 0xE0;

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

const fn make_table(pairs: &[(u8, char)]) -> [Option<char>; 128] {
    let mut table = [None; 128];
    let mut i = 0;
    while i < pairs.len() {
        let (scancode, ch) = pairs[i];
        table[scancode as usize] = Some(ch);
        i += 1;
    }
    table
}

const NORMAL_KEYS: [Option<char>; 128] = make_table(&[
    (0x01, '\x1B'), (0x02, '1'), (0x03, '2'), (0x04, '3'), (0x05, '4'),
    (0x06, '5'), (0x07, '6'), (0x08, '7'), (0x09, '8'), (0x0A, '9'),
    (0x0B, '0'), (0x0C, '-'), (0x0D, '='), (0x0F, '\t'),
    (0x10, 'q'), (0x11, 'w'), (0x12, 'e'), (0x13, 'r'), (0x14, 't'),
    (0x15, 'y'), (0x16, 'u'), (0x17, 'i'), (0x18, 'o'), (0x19, 'p'),
    (0x1A, '['), (0x1B, ']'), (0x1C, '\n'),
    (0x1E, 'a'), (0x1F, 's'), (0x20, 'd'), (0x21, 'f'), (0x22, 'g'),
    (0x23, 'h'), (0x24, 'j'), (0x25, 'k'), (0x26, 'l'), (0x27, ';'),
    (0x28, '\''), (0x29, '`'), (0x2B, '\\'),
    (0x2C, 'z'), (0x2D, 'x'), (0x2E, 'c'), (0x2F, 'v'), (0x30, 'b'),
    (0x31, 'n'), (0x32, 'm'), (0x33, ','), (0x34, '.'), (0x35, '/'),
    (0x39, ' '), (0x0E, '\x08'),
]);

const SHIFT_KEYS: [Option<char>; 128] = make_table(&[
    (0x01, '\x1B'), (0x02, '!'), (0x03, '@'), (0x04, '#'), (0x05, '$'),
    (0x06, '%'), (0x07, '^'), (0x08, '&'), (0x09, '*'), (0x0A, '('),
    (0x0B, ')'), (0x0C, '_'), (0x0D, '+'), (0x0F, '\t'),
    (0x10, 'Q'), (0x11, 'W'), (0x12, 'E'), (0x13, 'R'), (0x14, 'T'),
    (0x15, 'Y'), (0x16, 'U'), (0x17, 'I'), (0x18, 'O'), (0x19, 'P'),
    (0x1A, '{'), (0x1B, '}'), (0x1C, '\n'),
    (0x1E, 'A'), (0x1F, 'S'), (0x20, 'D'), (0x21, 'F'), (0x22, 'G'),
    (0x23, 'H'), (0x24, 'J'), (0x25, 'K'), (0x26, 'L'), (0x27, ':'),
    (0x28, '"'), (0x29, '~'), (0x2B, '|'),
    (0x2C, 'Z'), (0x2D, 'X'), (0x2E, 'C'), (0x2F, 'V'), (0x30, 'B'),
    (0x31, 'N'), (0x32, 'M'), (0x33, '<'), (0x34, '>'), (0x35, '?'),
    (0x39, ' '), (0x0E, '\x08'),
]);

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

pub fn init() {
    println!("[KEYBOARD] PS/2 init started...");
    unsafe {
        for _ in 0..1000 {
            if inb(PS2_STATUS_PORT) & 0x02 == 0 { break; }
            core::hint::spin_loop();
        }
        outb(0x64, 0xAE);
        for _ in 0..10000 { core::hint::spin_loop(); }
        println!("[KEYBOARD] Init complete.");
    }
}

/// Возвращает сырой скан-код, если он есть
pub fn poll_scancode() -> Option<u8> {
    unsafe {
        if inb(PS2_STATUS_PORT) & 0x01 == 0 { return None; }
        Some(inb(PS2_DATA_PORT))
    }
}

/// Возвращает логическую клавишу (с учётом Shift, extended keys)
pub fn poll_key() -> Option<Key> {
    let scancode = poll_scancode()?;

    unsafe {
        // Обработка extended prefix
        if scancode == SCANCODE_EXTENDED {
            EXTENDED_KEY = true;
            return None;
        }

        // Extended keys (стрелки)
        if EXTENDED_KEY {
            EXTENDED_KEY = false;
            let is_release = scancode & 0x80 != 0;
            if is_release { return None; }
            return Some(match scancode {
                0x4B => Key::Left,
                0x4D => Key::Right,
                0x48 => Key::Up,
                0x50 => Key::Down,
                _ => Key::Unknown,
            });
        }

        // Shift handling
        match scancode {
            SCANCODE_LSHIFT | SCANCODE_RSHIFT => { SHIFT_PRESSED = true; return None; }
            SCANCODE_LSHIFT_RELEASE | SCANCODE_RSHIFT_RELEASE => { SHIFT_PRESSED = false; return None; }
            _ => {}
        }

        // Ignore releases
        if scancode & 0x80 != 0 { return None; }

        let table = if SHIFT_PRESSED { &SHIFT_KEYS } else { &NORMAL_KEYS };
        if let Some(ch) = table[scancode as usize] {
            Some(match ch {
                '\n' => Key::Enter,
                '\x08' => Key::Backspace,
                '\t' => Key::Tab,
                _ => Key::Char(ch),
            })
        } else {
            None
        }
    }
}