use super::CmdResult;
use crate::framebuffer::Console;
use crate::fs;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print(&fs::get_current_dir());
    console.put_char('\n');
    CmdResult::Ok
}