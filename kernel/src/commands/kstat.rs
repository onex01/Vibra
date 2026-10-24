use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_RED};
use alloc::format;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let kb_count = crate::keyboard::irq_count();
    let ticks = crate::interrupts::idt::ticks();
    let uptime_ms = crate::timer::uptime_ms();
    let measured_hz = crate::timer::measure_irq_hz(200);

    console.print_colored("=== Interrupt / Timer Statistics ===\n", COLOR_YELLOW);

    console.print("  Uptime:        ");
    console.print(&format!("{}.{:03} s\n", uptime_ms / 1000, uptime_ms % 1000));

    console.print("  Clock source:  ");
    console.print(&format!(
        "{} ({} Hz)\n",
        crate::timer::current_source().name(),
        crate::timer::source_hz()
    ));

    console.print("  Timer ticks:   ");
    console.print(&format!("{}\n", ticks));

    console.print("  IRQ tick rate: ");
    console.print(&format!(
        "{} Hz (expected {})\n",
        measured_hz,
        crate::interrupts::idt::TIMER_HZ
    ));

    console.print("  KB IRQ count:  ");
    console.print(&format!("{}\n", kb_count));

    if kb_count == 0 {
        console.print_colored("  WARNING: No keyboard interrupts received!\n", COLOR_RED);
        console.print("  Possible causes:\n");
        console.print("    1. QEMU window not focused\n");
        console.print("    2. PIC not delivering IRQ1 (polling fallback still works)\n");
        console.print("    3. IDT[33] handler not installed\n");
    } else {
        console.print_colored("  Keyboard interrupts OK\n", COLOR_GREEN);
    }

    if ticks == 0 {
        console.print_colored(
            "  WARNING: No timer ticks — IRQ0 is not delivered.\n",
            COLOR_RED,
        );
        console.print("  Uptime/date use the hardware clock (TSC/HPET), so time stays correct.\n");
    } else {
        console.print_colored("  Timer ticks OK\n", COLOR_GREEN);
    }

    if !crate::timer::irq_ticks_healthy() {
        console.print_colored(
            "  Idle path: busy-poll (hlt would never return without IRQ ticks)\n",
            COLOR_YELLOW,
        );
    }

    CmdResult::Ok
}
