// log — вывод boot-лога на экран и управление приёмником.
//
// Сам лог живёт в кольцевом буфере (`boot_log`): на машине без COM-порта это
// единственный способ увидеть, что произошло до shell. Здесь печатаются
// последние строки — целиком 128 КиБ в консоль бессмысленно.
//
// Подкоманды:
//   log            — последние 40 строк
//   log <N>        — последние N строк
//   log flush      — принудительный checkpoint (ramfs + VIBRALOG)
//   log sink       — куда сейчас пишется лог
//   log path       — путь файла-приёмника

use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use super::CmdResult;
use alloc::format;
use alloc::string::String;

const DEFAULT_LINES: usize = 40;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    match args.first().copied() {
        Some("flush") => return flush(console),
        Some("sink") => return sink(console),
        Some("path") => return path(console),
        _ => {}
    }

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

/// Принудительный сброс лога на все приёмники.
fn flush(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] flushing...\n", COLOR_CYAN);

    let before = crate::boot_log::usb::written();
    crate::boot_log::flush_to_file("/var/log/boot.log");
    crate::boot_log::checkpoint();
    let after = crate::boot_log::usb::written();

    console.print_colored("  ramfs: ", COLOR_GREEN);
    console.print("/var/log/boot.log\n");

    if crate::boot_log::usb::is_attached() {
        console.print_colored("  VIBRALOG: ", COLOR_GREEN);
        console.print(&format!(
            "{} bytes total ({} written by this flush)\n",
            after,
            after.saturating_sub(before)
        ));
    } else {
        console.print_colored("  VIBRALOG: ", COLOR_YELLOW);
        console.print("not attached (no FAT32 volume labelled VIBRALOG)\n");
    }

    CmdResult::Ok
}

/// Куда сейчас пишется лог.
fn sink(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] sinks\n", COLOR_CYAN);
    console.print(&format!(
        "  ramfs    : /var/log/boot.log ({} bytes)\n",
        crate::boot_log::total_len()
    ));

    if crate::boot_log::usb::is_attached() {
        console.print_colored("  VIBRALOG : ", COLOR_GREEN);
        console.print(&format!(
            "/{} ({} bytes written)\n",
            crate::boot_log::usb::LOG_FILE,
            crate::boot_log::usb::written()
        ));
    } else {
        console.print_colored("  VIBRALOG : ", COLOR_YELLOW);
        console.print("not attached\n");
    }

    console.print_colored("  debugcon : ", COLOR_GREEN);
    console.print("port 0xE9 ([TICK]/[HALT]/[HEARTBEAT])\n");

    CmdResult::Ok
}

/// Путь файла-приёмника на носителе.
fn path(console: &mut Console) -> CmdResult {
    if crate::boot_log::usb::is_attached() {
        console.print_colored("VIBRALOG:/", COLOR_GREEN);
        console.print(crate::boot_log::usb::LOG_FILE);
        console.print("\n");
    } else {
        console.print_colored(
            "no VIBRALOG volume; log is only in ramfs (/var/log/boot.log)\n",
            COLOR_RED,
        );
    }
    CmdResult::Ok
}
