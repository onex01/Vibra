use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(path) = args.first() else {
        console.print_colored("Usage: cd <directory>\n", COLOR_RED);
        return CmdResult::Ok;
    };

    match *path {
        "/" => {
            fs::set_current_dir("/");
            console.print_colored("Changed to: /\n", COLOR_GREEN);
            return CmdResult::Ok;
        }
        ".." => {
            // Пока просто остаёмся в корне (полная навигация будет позже)
            fs::set_current_dir("/");
            console.print_colored("Changed to: ..\n", COLOR_GREEN);
            return CmdResult::Ok;
        }
        _ => {}
    }

    if fs::dir_exists(path) {
        fs::set_current_dir(path);
        console.print_colored("Changed to: ", COLOR_GREEN);
        console.print(path);
        console.put_char('\n');
    } else {
        console.print_colored("Error: directory '", COLOR_RED);
        console.print(path);
        console.print_colored("' not found\n", COLOR_RED);
    }

    CmdResult::Ok
}