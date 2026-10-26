// log — вывод boot-лога на экран.
//
// Сам лог живёт в кольцевом буфере (`boot_log`): на машине без COM-порта это
// единственный способ увидеть, что произошло до shell. Здесь печатаются
// последние строки — целиком 128 КиБ в консоль бессмысленно.

use crate::framebuffer::{Console, COLOR_YELLOW};
use super::CmdResult;
use alloc::format;
use alloc::string::String;

const DEFAULT_LINES: usize = 40;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let lines = args
        .first()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LINES)
        .max(1);

    let raw = crate::boot_log::get_log_bytes();
    if raw.is_empty() {
        console.print_colored("boot log is empty\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    let text = String::from_utf8_lossy(&raw);
    let all: alloc::vec::Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);

    console.print_colored("[BOOT LOG]\n", COLOR_YELLOW);
    console.print(&format!(
        "  {} bytes, {} lines total; sink: {}{}\n",
        raw.len(),
        all.len(),
        if crate::boot_log::usb::is_attached() {
            "VIBRALOG"
        } else {
            "ramfs only"
        },
        if crate::boot_log::usb::is_attached() {
            format!(
                " ({} bytes written)",
                crate::boot_log::usb::written()
            )
        } else {
            String::new()
        }
    ));

    for line in &all[start..] {
        console.print(line);
        console.print("\n");
    }

    CmdResult::Ok
}
