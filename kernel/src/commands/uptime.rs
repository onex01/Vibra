use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};
use alloc::format;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let ms = crate::timer::uptime_ms();
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;

    console.print_colored("System Uptime:\n", COLOR_CYAN);
    console.print_colored("  Uptime:      ", COLOR_YELLOW);
    console.print(&format!(
        "{}d {:02}h {:02}m {:02}s ({}.{:03} s)\n",
        days,
        hours % 24,
        mins % 60,
        secs % 60,
        secs,
        ms % 1000
    ));

    console.print_colored("  Clock:       ", COLOR_YELLOW);
    console.print(&format!(
        "{} ({} Hz)\n",
        crate::timer::current_source().name(),
        crate::timer::source_hz()
    ));

    console.print_colored("  IRQ ticks:   ", COLOR_YELLOW);
    console.print(&format!(
        "{} ({})\n",
        crate::interrupts::idt::ticks(),
        if crate::timer::irq_ticks_healthy() {
            "delivered"
        } else {
            "not delivered — time comes from the hardware clock"
        }
    ));

    console.print_colored("  Wall clock:  ", COLOR_YELLOW);
    match crate::timer::wall_clock() {
        Some(dt) => console.print(&format!("{}\n", dt.format())),
        None => console.print("RTC unavailable\n"),
    }

    CmdResult::Ok
}
