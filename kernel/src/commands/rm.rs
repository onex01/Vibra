use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(name) = args.first() else {
        console.print_colored("Usage: rm <name>\n", COLOR_RED);
        return CmdResult::Ok;
    };
    match fs::remove_entry(name) {
        Ok(_) => { console.print_colored("Removed: ", COLOR_GREEN); console.print(name); console.put_char('\n'); }
        Err(e) => { console.print_colored("Error: ", COLOR_RED); console.print(&format!("{}", e)); console.put_char('\n'); }
    }
    CmdResult::Ok
}