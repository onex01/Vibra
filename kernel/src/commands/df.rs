use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_CYAN, COLOR_GREEN};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Filesystem      Size     Used    Avail   Mount\n", COLOR_CYAN);
    console.print_colored("-----------     ------   ------  ------  -------\n", COLOR_CYAN);

    // Root filesystem (RamFS)
    let (used, total) = crate::memory::heap::stats();
    let used_kb = used / 1024;
    let total_kb = total / 1024;
    let free_kb = total_kb - used_kb;

    console.print_colored("ramfs           ", COLOR_GREEN);
    console.print_num(total_kb);
    console.print("KB   ");
    console.print_num(used_kb);
    console.print("KB   ");
    console.print_num(free_kb);
    console.print("KB   /\n");

    // /proc (virtual)
    console.print_colored("procfs          ", COLOR_GREEN);
    console.print("0KB     0KB     0KB     /proc\n");

    // /sys (virtual)
    console.print_colored("sysfs           ", COLOR_GREEN);
    console.print("0KB     0KB     0KB     /sys\n");

    // /dev (virtual)
    console.print_colored("devtmpfs        ", COLOR_GREEN);
    console.print("0KB     0KB     0KB     /dev\n");

    console.print("\n");
    console.print_colored("Heap: ", COLOR_YELLOW);
    console.print("used=");
    console.print_num(used);
    console.print(" total=");
    console.print_num(total);
    console.print("\n");

    // PMM stats
    let (used_frames, total_frames) = crate::memory::pmm::stats();
    let used_mb = (used_frames * 4096) / (1024 * 1024);
    let total_mb = (total_frames * 4096) / (1024 * 1024);

    console.print_colored("RAM:  ", COLOR_YELLOW);
    console.print_num(used_mb);
    console.print("MB / ");
    console.print_num(total_mb);
    console.print("MB used\n");

    CmdResult::Ok
}
