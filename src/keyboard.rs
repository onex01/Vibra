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

struct KeyboardState {
    shift_pressed: bool,
    extended_key: bool,
    buffer: [u8; 32],
    head: usize,
    tail: usize,
}

impl KeyboardState {
    const fn new() -> Self {
        KeyboardState {
            shift_pressed: false,
            extended_key: false,
            buffer: [0; 32],
            head: 0,
            tail: 0,
        }
    }
    
    fn push_scancode(&mut self, scancode: u8) {
        let next = (self.head + 1) % 32;
        if next != self.tail {
            self.buffer[self.head] = scancode;
            self.head = next;
        }
    }
    
    fn pop_scancode(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let sc = self.buffer[self.tail];
            self.tail = (self.tail + 1) % 32;
            Some(sc)
        }
    }
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

const SCANCODE_LSHIFT: u8 = 0x2A;
const SCANCODE_RSHIFT: u8 = 0x36;
const SCANCODE_LSHIFT_RELEASE: u8 = 0xAA;
const SCANCODE_RSHIFT_RELEASE: u8 = 0xB6;
const SCANCODE_EXTENDED: u8 = 0xE0;

// Используем Mutex вместо static mut
static KEYBOARD_STATE: Mutex<KeyboardState> = Mutex::new(KeyboardState::new());

pub fn init() {
    println!("[KEYBOARD] PS/2 keyboard ready (interrupt-driven)");
}

pub fn handle_interrupt(scancode: u8) {
    let mut state = KEYBOARD_STATE.lock();
    
    state.push_scancode(scancode);
    
    match scancode {
        SCANCODE_LSHIFT | SCANCODE_RSHIFT => state.shift_pressed = true,
        SCANCODE_LSHIFT_RELEASE | SCANCODE_RSHIFT_RELEASE => state.shift_pressed = false,
        _ => {}
    }
}

pub fn poll_key() -> Option<Key> {
    let mut state = KEYBOARD_STATE.lock();
    let scancode = state.pop_scancode()?;
    
    if scancode == SCANCODE_EXTENDED {
        state.extended_key = true;
        return None;
    }
    
    if state.extended_key {
        state.extended_key = false;
        if scancode & 0x80 != 0 { return None; }
        return Some(match scancode {
            0x4B => Key::Left,
            0x4D => Key::Right,
            0x48 => Key::Up,
            0x50 => Key::Down,
            _ => Key::Unknown,
        });
    }
    
    if scancode & 0x80 != 0 { return None; }
    
    let table = if state.shift_pressed { &SHIFT_KEYS } else { &NORMAL_KEYS };
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