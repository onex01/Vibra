use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("/\n");
    CmdResult::Ok
}