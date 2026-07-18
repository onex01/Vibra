use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("System halting...\n", COLOR_RED);
    CmdResult::Exit
}