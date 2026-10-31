use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_WHITE};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let mem = crate::memory::meminfo();

    console.print_colored(
        "               total          used          free        used%\n",
        COLOR_CYAN,
    );

    console.print_colored("Mem:  ", COLOR_WHITE);
    print_cell(console, &crate::memory::format_bytes(mem.total_bytes));
    print_cell(console, &crate::memory::format_bytes(mem.used_bytes));
    print_cell(console, &crate::memory::format_bytes(mem.free_bytes));
    console.print(&alloc::format!("{:>5}%\n", mem.used_percent()));

    console.print_colored("Heap: ", COLOR_WHITE);
    let heap_pct = if mem.heap_total > 0 {
        (mem.heap_used * 100) / mem.heap_total
    } else {
        0
    };
    print_cell(console, &crate::memory::format_bytes(mem.heap_total));
    print_cell(console, &crate::memory::format_bytes(mem.heap_used));
    print_cell(
        console,
        &crate::memory::format_bytes(mem.heap_total.saturating_sub(mem.heap_used)),
    );
    console.print(&alloc::format!("{:>5}%\n", heap_pct));

    console.print_colored("Swap: ", COLOR_WHITE);
    print_cell(console, "0 B");
    print_cell(console, "0 B");
    print_cell(console, "0 B");
    console.print("    0%\n");

    console.put_char('\n');
    console.print_colored("Physical RAM: ", COLOR_GREEN);
    console.print(&crate::memory::format_bytes(mem.total_bytes));
    console.print(", free ");
    console.print(&crate::memory::format_bytes(mem.free_bytes));
    console.put_char('\n');

    CmdResult::Ok
}

fn print_cell(console: &mut Console, text: &str) {
    // Колонки фиксированной ширины 15: значения вида "15.6 GiB".
    let width = 15usize;
    let len = text.len();
    if len < width {
        for _ in 0..(width - len) {
            console.put_char(' ');
        }
    }
    console.print(text);
    console.print("  ");
}
