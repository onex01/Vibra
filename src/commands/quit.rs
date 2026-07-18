use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    CmdResult::Exit
}