use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_VIBRA_FG};

// heap — показать использование кучи.
// Выводит used/total/free байты из free-list аллокатора.
pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let (used, total) = crate::memory::heap::stats();
    let free = total.saturating_sub(used);

    let kb = |b: usize| b / 1024;
    let pct = if total != 0 { (used * 100) / total } else { 0 };

    console.print_colored("Heap (free-list allocator)\n", COLOR_CYAN);
    console.print_colored(
        &alloc::format!(
            "  used:  {} bytes ({} KB)\n",
            used, kb(used)
        ),
        COLOR_VIBRA_FG,
    );
    console.print_colored(
        &alloc::format!(
            "  free:  {} bytes ({} KB)\n",
            free, kb(free)
        ),
        COLOR_GREEN,
    );
    console.print_colored(
        &alloc::format!(
            "  total: {} bytes ({} KB)  [{}% used]\n",
            total, kb(total), pct
        ),
        COLOR_VIBRA_FG,
    );
    CmdResult::Ok
}
