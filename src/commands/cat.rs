use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(name) = args.first() else {
        console.print_colored("Usage: cat <filename>\n", COLOR_RED);
        return CmdResult::Ok;
    };
    match fs::read_file(name) {
        Ok(data) => {
            if let Ok(text) = core::str::from_utf8(data) {
                console.print(text);
                if !text.ends_with('\n') { console.put_char('\n'); }
            } else {
                console.print_colored("(binary data, ", COLOR_RED);
                console.print_num(data.len());
                console.print(" bytes)\n");
            }
        }
        Err(e) => { console.print_colored("Error: ", COLOR_RED); console.print(e); console.put_char('\n'); }
    }
    CmdResult::Ok
}