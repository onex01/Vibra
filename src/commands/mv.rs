use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.len() < 2 {
        console.print_colored("Usage: mv <source> <destination>\n", COLOR_RED);
        return CmdResult::Ok;
    }

    let source = args[0];
    let dest = args[1];

    match fs::read_file(source) {
        Ok(data) => {
            // Если файл назначения существует, удаляем его
            let _ = fs::remove_entry(dest);
            
            // Создаём и записываем
            if fs::create_file(dest).is_ok() {
                if let Err(e) = fs::write_file(dest, &data) {
                    console.print_colored("Error writing: ", COLOR_RED);
                    console.print(e);
                    console.put_char('\n');
                    return CmdResult::Ok;
                }
                
                // Удаляем исходный файл
                if let Err(e) = fs::remove_entry(source) {
                    console.print_colored("Error removing source: ", COLOR_RED);
                    console.print(e);
                    console.put_char('\n');
                    return CmdResult::Ok;
                }
                
                console.print_colored("Moved: ", COLOR_GREEN);
                console.print(source);
                console.print_colored(" -> ", COLOR_YELLOW);
                console.print(dest);
                console.put_char('\n');
            } else {
                console.print_colored("Error creating destination\n", COLOR_RED);
            }
        }
        Err(e) => {
            console.print_colored("Error: ", COLOR_RED);
            console.print(e);
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}