use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.len() < 2 {
        console.print_colored("Usage: cp <source> <destination>\n", COLOR_RED);
        return CmdResult::Ok;
    }

    let source = args[0];
    let dest = args[1];

    // Копирование потоком: файл не загружается в память целиком.
    match fs::copy_file(source, dest) {
        Ok(bytes) => {
            console.print_colored("Copied: ", COLOR_GREEN);
            console.print(source);
            console.print_colored(" -> ", COLOR_YELLOW);
            console.print(dest);
            console.print(" (");
            console.print_num(bytes as usize);
            console.print(" bytes)\n");
        }
        Err(e) => {
            console.print_colored("Error copying: ", COLOR_RED);
            console.print(&format!("{}", e));
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}
