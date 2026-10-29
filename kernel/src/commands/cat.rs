use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() || args.iter().any(|a| *a == "--help" || *a == "-h") {
        console.print_colored("Usage: cat <file> [file...]\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    for name in args {
        match fs::read_file(name) {
            Ok(data) => {
                if let Ok(text) = core::str::from_utf8(&data) {
                    console.print(text);
                    if !text.ends_with('\n') {
                        console.put_char('\n');
                    }
                } else {
                    console.print_colored("(binary data, ", COLOR_RED);
                    console.print_num(data.len());
                    console.print(" bytes)\n");
                }
            }
            Err(e) => {
                console.print_colored("cat: ", COLOR_RED);
                console.print(name);
                console.print(": ");
                console.print(&format!("{}", e));
                console.put_char('\n');
            }
        }
    }

    CmdResult::Ok
}