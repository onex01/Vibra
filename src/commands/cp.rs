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

    // Читаем исходный файл
    match fs::read_file(source) {
        Ok(data) => {
            // Создаём копию (если файл существует — перезаписываем)
            // Сначала пробуем создать
            if fs::create_file(dest).is_err() {
                // Файл уже существует — это нормально, просто перезапишем
            }
            
            // Записываем данные
            match fs::write_file(dest, &data) {
                Ok(_) => {
                    console.print_colored("Copied: ", COLOR_GREEN);
                    console.print(source);
                    console.print_colored(" -> ", COLOR_YELLOW);
                    console.print(dest);
                    console.print(" (");
                    console.print_num(data.len());
                    console.print(" bytes)\n");
                }
                Err(e) => {
                    console.print_colored("Error writing: ", COLOR_RED);
                    console.print(&format!("{}", e));
                    console.put_char('\n');
                }
            }
        }
        Err(e) => {
            console.print_colored("Error reading: ", COLOR_RED);
            console.print(&format!("{}", e));
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}