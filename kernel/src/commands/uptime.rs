use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};
use crate::interrupts;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let ticks = interrupts::idt::ticks();
    // PIT запрограммирован на TIMER_HZ (см. idt.rs)
    let seconds = ticks / interrupts::idt::TIMER_HZ;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    
    console.print_colored("System Uptime:\n", COLOR_CYAN);
    console.print_colored("  Timer ticks: ", COLOR_YELLOW);
    console.print_num(ticks as usize);
    console.print("\n");
    
    console.print_colored("  Uptime:      ", COLOR_YELLOW);
    console.print_num(hours as usize);
    console.print("h ");
    console.print_num((minutes % 60) as usize);
    console.print("m ");
    console.print_num((seconds % 60) as usize);
    console.print("s\n");
    
    CmdResult::Ok
}