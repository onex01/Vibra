use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN};
use crate::fs;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Disk Usage (Vibra 0.4)\n", COLOR_CYAN);
    console.print_colored("======================\n", COLOR_CYAN);

    let count = fs::fs_count();
    let max_entries = 64usize;
    let mut total_size = 0usize;
    for entry in fs::list_entries() {
        total_size += entry.size;
    }

    console.print("Device : ramfs://\n");
    console.print("Type   : RamFS (in-memory)\n\n");

    console.print_colored("Entries : ", COLOR_YELLOW);
    console.print_num(count);
    console.print(" / ");
    console.print_num(max_entries);
    console.print("\n");

    console.print_colored("Used    : ", COLOR_YELLOW);
    console.print_num(total_size);
    console.print(" bytes\n");

    console.print_colored("Capacity: ", COLOR_YELLOW);
    console.print_num(max_entries * 4096);
    console.print(" bytes (max)\n");

    console.print_colored("Status  : ", COLOR_GREEN);
    console.print("OK\n");
    CmdResult::Ok
}