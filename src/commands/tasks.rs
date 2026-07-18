use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Task Manager (Vibra 0.4)\n", COLOR_CYAN);
    console.print_colored("========================\n", COLOR_CYAN);
    console.print_colored("PID  STATE     NAME\n", COLOR_YELLOW);
    console.print("  0  RUNNING   kernel (idle)\n");
    console.print("  1  ACTIVE    shell\n");
    console.print_colored("\n(Multitasking not yet implemented)\n", COLOR_CYAN);
    CmdResult::Ok
}