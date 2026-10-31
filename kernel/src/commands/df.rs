use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_DARK_GRAY, COLOR_GREEN, COLOR_RED, COLOR_WHITE, COLOR_YELLOW};
use alloc::format;

/// df — использование смонтированных файловых систем.
///
/// Раньше команда печатала выдуманные размеры: размер ramfs брался из
/// статистики heap, а procfs/sysfs/devtmpfs всегда показывали 0. Теперь
/// источник — реальная таблица монтирования; размер известен только для
/// ramfs (он живёт в heap), у остальных стоит `-`.
pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let mem = crate::memory::meminfo();

    console.print_colored(
        "FILESYSTEM   SIZE        USED        AVAIL       USE%  MODE  MOUNT\n",
        COLOR_CYAN,
    );
    console.print_colored(
        "----------   ----------  ----------  ----------  ----  ----  ----------\n",
        COLOR_DARK_GRAY,
    );

    let entries = crate::fs::mount_entries();
    if entries.is_empty() {
        console.print_colored("  No filesystems mounted\n", COLOR_RED);
        return CmdResult::Ok;
    }

    for (path, fs_name, readonly) in entries {
        // Размер есть только у ramfs: остальные ФС не отдают статистику.
        let (size, used, avail) = if fs_name == "ramfs" {
            (
                crate::memory::format_bytes(mem.heap_total),
                crate::memory::format_bytes(mem.heap_used),
                crate::memory::format_bytes(mem.heap_total.saturating_sub(mem.heap_used)),
            )
        } else {
            (
                alloc::string::String::from("-"),
                alloc::string::String::from("-"),
                alloc::string::String::from("-"),
            )
        };

        let pct = if fs_name == "ramfs" && mem.heap_total > 0 {
            format!("{}%", (mem.heap_used * 100) / mem.heap_total)
        } else {
            alloc::string::String::from("-")
        };

        console.print(&format!("{:<12} ", fs_name));
        console.print(&format!("{:<11} ", size));
        console.print(&format!("{:<11} ", used));
        console.print(&format!("{:<11} ", avail));
        console.print(&format!("{:<5} ", pct));
        if readonly {
            console.print_colored("ro    ", COLOR_YELLOW);
        } else {
            console.print_colored("rw    ", COLOR_GREEN);
        }
        console.print(&path);
        console.put_char('\n');
    }

    console.put_char('\n');
    console.print_colored("Physical RAM: ", COLOR_WHITE);
    console.print(&crate::memory::format_pair(mem.used_bytes, mem.total_bytes));
    console.print(" (");
    console.print_num(mem.used_percent() as usize);
    console.print("%)\n");
    console.print_colored(
        "Block devices: `disks`. Mount: `mount <path> auto <dev>`.\n",
        COLOR_DARK_GRAY,
    );

    CmdResult::Ok
}
