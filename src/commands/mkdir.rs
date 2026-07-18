use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(name) = args.first() else {
        console.print_colored("Usage: mkdir <dirname>\n", COLOR_RED);
        return CmdResult::Ok;
    };
    match fs::create_dir(name) {
        Ok(_) => { console.print_colored("Created: ", COLOR_GREEN); console.print(name); console.put_char('\n'); }
        Err(e) => { console.print_colored("Error: ", COLOR_RED); console.print(e); console.put_char('\n'); }
    }
    CmdResult::Ok
}