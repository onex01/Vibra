// disks — список блочных устройств и разделов.
//
// Нужен для диагностики «диск виден, но ФС unknown»: показывает имя,
// вид, размер, смещение в байтах, определённый тип ФС и метку раздела.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_DARK_GRAY, COLOR_GREEN, COLOR_RED, COLOR_WHITE, COLOR_YELLOW};
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let verbose = args.iter().any(|a| *a == "-v" || *a == "--verbose");

    let mgr = crate::fs::get_disk_manager().lock();

    console.print_colored("NAME      TYPE       SIZE      OFFSET     FS\n", COLOR_CYAN);
    // Только ASCII: символы U+2500 нет в шрифте консоли, и строка
    // разделителя рисовалась пустой.
    console.print_colored(
        "--------  ---------  --------  ---------  ------------\n",
        COLOR_DARK_GRAY,
    );

    if mgr.count() == 0 {
        console.print_colored("  No block devices found\n", COLOR_RED);
        return CmdResult::Ok;
    }

    for entry in mgr.entries() {
        console.print(&format!("{:<9}", entry.name));
        console.print(" ");

        if entry.kind == "disk" {
            console.print_colored(&format!("{:<9}", entry.kind), COLOR_WHITE);
        } else {
            console.print_colored(&format!("{:<9}", entry.kind), COLOR_YELLOW);
        }
        console.print(" ");

        let size = if entry.size_mb() >= 1024 {
            // tenths of GiB: 1024 * 10 / 1024 → делим остаток на 102.4
            let tenths = (entry.size_mb() % 1024) * 10 / 1024;
            format!("{}.{} GiB", entry.size_mb() / 1024, tenths)
        } else {
            format!("{} MiB", entry.size_mb())
        };
        console.print(&format!("{:<9}", size));
        console.print(" ");

        console.print(&format!("{:<10}", entry.offset_bytes()));
        console.print(" ");

        match entry.fs_type {
            "unknown" => console.print_colored(entry.fs_type, COLOR_RED),
            "exfat" => console.print_colored(entry.fs_type, COLOR_YELLOW),
            _ => console.print_colored(entry.fs_type, COLOR_GREEN),
        }
        console.put_char('\n');

        if verbose && entry.sectors > 0 {
            console.print_colored("          sectors: ", COLOR_DARK_GRAY);
            console.print_num(entry.sectors as usize);
            console.put_char('\n');
        }
    }

    let (read, write) = crate::fs::disk::io_stats();
    console.put_char('\n');
    console.print_colored("I/O: read ", COLOR_WHITE);
    console.print(&crate::memory::format_bytes(read));
    console.print(", write ");
    console.print(&crate::memory::format_bytes(write));
    console.put_char('\n');

    console.print_colored(
        "\nMount: mount <path> auto <name>\n",
        COLOR_DARK_GRAY,
    );

    CmdResult::Ok
}
