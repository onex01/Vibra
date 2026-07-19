use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let ticks = crate::interrupts::idt::ticks();
    let secs = ticks / 100;
    let mins = secs / 60;
    let hours = mins / 60;

    // Упрощённый формат: HH:MM:SS
    console.print_num(hours as usize);
    console.print(":");
    if mins % 60 < 10 { console.print("0"); }
    console.print_num((mins % 60) as usize);
    console.print(":");
    if secs % 60 < 10 { console.print("0"); }
    console.print_num((secs % 60) as usize);
    console.print(" UTC\n");
    CmdResult::Ok
}
