// USB HID boot keyboard: разбор 8-байтовых отчётов.
//
// Boot protocol report:
//   [0] modifiers (bit0 LCtrl, bit1 LShift, bit2 LAlt, bit3 LGui,
//                  bit4 RCtrl, bit5 RShift, bit6 RAlt, bit7 RGui)
//   [1] reserved
//   [2..8] usage ID нажатых клавиш (до 6 одновременно)
//
// Драйвер xHCI вызывает `handle_boot_report` на каждый Transfer Event.

use crate::keyboard::Key as KKey;
use crate::println;
use spin::Mutex;

static LAST_REPORT: Mutex<[u8; 6]> = Mutex::new([0; 6]);

pub fn init() {
    println!("[HID] USB HID boot keyboard support ready");
}

/// Обрабатывает отчёт boot-клавиатуры и инжектит новые нажатия в keyboard.
pub fn handle_boot_report(report: &[u8]) {
    if report.len() < 8 {
        return;
    }

    let modifiers = report[0];
    let shift = modifiers & 0x22 != 0; // Left/Right Shift

    let mut current = [0u8; 6];
    current.copy_from_slice(&report[2..8]);

    let mut previous = LAST_REPORT.lock();
    for &usage in &current {
        if usage == 0 || previous.contains(&usage) {
            continue;
        }
        if let Some(key) = translate(usage, shift) {
            crate::keyboard::inject_key(key);
        }
    }
    *previous = current;
}

fn translate(usage: u8, shift: bool) -> Option<KKey> {
    let ch = match usage {
        0x04..=0x1D => (b'a' + (usage - 0x04)) as char,
        0x1E..=0x26 => (b'1' + (usage - 0x1E)) as char,
        0x27 => '0',
        0x28 => return Some(KKey::Enter),
        0x29 => '\x1B',
        0x2A => return Some(KKey::Backspace),
        0x2B => return Some(KKey::Tab),
        0x2C => ' ',
        0x2D => '-',
        0x2E => '=',
        0x2F => '[',
        0x30 => ']',
        0x31 => '\\',
        0x32 => ';',
        0x33 => '\'',
        0x34 => '`',
        0x35 => ',',
        0x36 => '.',
        0x37 => '/',
        0x4F => return Some(KKey::Right),
        0x50 => return Some(KKey::Left),
        0x51 => return Some(KKey::Down),
        0x52 => return Some(KKey::Up),
        _ => return None,
    };

    let ch = if shift { crate::keyboard::shift_symbol(ch) } else { ch };
    Some(KKey::Char(ch))
}
