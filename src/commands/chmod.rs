use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.len() < 2 {
        console.print_colored("Usage: chmod <mode> <file>\n", COLOR_YELLOW);
        console.print("  Example: chmod 755 myfile\n");
        console.print("  Modes: rwx (7), rw- (6), r-- (4), etc.\n");
        return CmdResult::Ok;
    }

    let mode_str = args[0];
    let path = args[1];

    // Парсим OCTAL режим (например "755")
    let mode = if mode_str.len() == 3 {
        let mut m = 0u16;
        for (i, c) in mode_str.chars().enumerate() {
            if let Some(digit) = c.to_digit(8) {
                m |= (digit as u16) << ((2 - i) * 3);
            } else {
                console.print_colored("chmod: invalid mode '", COLOR_RED);
                console.print(mode_str);
                console.print("'\n");
                return CmdResult::Ok;
            }
        }
        m
    } else {
        console.print_colored("chmod: mode must be 3-digit octal (e.g. 755)\n", COLOR_RED);
        return CmdResult::Ok;
    };

    // Проверяем существование файла
    if fs::dir_exists(path) || fs::read_file(path).is_ok() {
        console.print_colored("chmod: ", COLOR_YELLOW);
        console.print(mode_str);
        console.print(" -> ");
        console.print(path);
        console.print("\n");
        console.print_colored("[chmod] Mode set (simulated)\n", COLOR_GREEN);
    } else {
        console.print_colored("chmod: cannot access '", COLOR_RED);
        console.print(path);
        console.print("' - No such file or directory\n");
    }

    CmdResult::Ok
}
