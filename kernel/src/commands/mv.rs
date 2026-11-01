use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.len() < 2 {
        console.print_colored("Usage: mv <source> <destination>\n", COLOR_RED);
        return CmdResult::Ok;
    }

    let source = args[0];
    let dest = args[1];

    // Потоковое копирование вместо чтения файла целиком: `mv` большого
    // файла раньше исчерпывал кучу и ронял ядро.
    let _ = fs::remove_entry(dest);
    match fs::copy_file(source, dest) {
        Ok(_) => {
            if let Err(e) = fs::remove_entry(source) {
                console.print_colored("Error removing source: ", COLOR_RED);
                console.print(&format!("{}", e));
                console.put_char('\n');
                return CmdResult::Ok;
            }
            console.print_colored("Moved: ", COLOR_GREEN);
            console.print(source);
            console.print_colored(" -> ", COLOR_YELLOW);
            console.print(dest);
            console.put_char('\n');
        }
        Err(e) => {
            console.print_colored("Error: ", COLOR_RED);
            console.print(&format!("{}", e));
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}
