use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_RED};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let kb_count = crate::keyboard::irq_count();
    let timer_ticks = crate::interrupts::idt::ticks();

    console.print_colored("=== Interrupt Statistics ===\n", COLOR_YELLOW);
    console.print("  Timer ticks:  ");
    console.print_num(timer_ticks as usize);
    console.print("\n");
    console.print("  KB IRQ count: ");
    console.print_num(kb_count as usize);
    console.print("\n");

    if kb_count == 0 {
        console.print_colored("  WARNING: No keyboard interrupts received!\n", COLOR_RED);
        console.print("  Possible causes:\n");
        console.print("    1. QEMU window not focused\n");
        console.print("    2. PIC not delivering IRQ1\n");
        console.print("    3. IDT[33] handler not installed\n");
    } else {
        console.print_colored("  Keyboard interrupts OK\n", COLOR_GREEN);
    }

    if timer_ticks == 0 {
        console.print_colored("  WARNING: No timer ticks!\n", COLOR_RED);
    } else {
        console.print_colored("  Timer OK\n", COLOR_GREEN);
    }

    CmdResult::Ok
}
