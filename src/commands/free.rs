use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_WHITE, COLOR_CYAN};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let (used_frames, total_frames) = crate::memory::pmm::stats();
    let (heap_used, heap_total) = crate::memory::heap::stats();

    let total_kb = (total_frames * 4096) / 1024;
    let used_kb = (used_frames * 4096) / 1024;
    let free_kb = total_kb - used_kb;

    console.print_colored("              total        used        free\n", COLOR_CYAN);
    console.print_colored("Mem:          ", COLOR_WHITE);
    print_kb(console, total_kb);
    console.print("    ");
    print_kb(console, used_kb);
    console.print("    ");
    print_kb(console, free_kb);
    console.print("\n\n");

    let total_mb = total_frames * 4096 / (1024 * 1024);
    let used_mb = used_frames * 4096 / (1024 * 1024);
    let free_mb = total_mb - used_mb;

    console.print_colored("Physical RAM: ", COLOR_GREEN);
    console.print_num(total_mb);
    console.print(" MB total, ");
    console.print_num(used_mb);
    console.print(" MB used, ");
    console.print_num(free_mb);
    console.print(" MB free\n");

    console.print_colored("Heap:         ", COLOR_GREEN);
    console.print_num(heap_used / 1024);
    console.print(" KB used, ");
    console.print_num(heap_total / 1024);
    console.print(" KB total\n");

    CmdResult::Ok
}

fn print_kb(console: &mut Console, kb: usize) {
    if kb < 10 {
        console.print("      ");
    } else if kb < 100 {
        console.print("     ");
    } else if kb < 1000 {
        console.print("    ");
    } else if kb < 10000 {
        console.print("   ");
    } else if kb < 100000 {
        console.print("  ");
    } else {
        console.print(" ");
    }
    console.print_num(kb);
}
