use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN};
use crate::fs;
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        // Показать текущее имя хоста
        match fs::read_file("/cfg/hostname") {
            Ok(data) => {
                if let Ok(s) = core::str::from_utf8(&data) {
                    console.print(s.trim_end());
                    console.put_char('\n');
                }
            }
            Err(_) => {
                console.print("vibra\n");
            }
        }
    } else {
        // Установить новое имя хоста
        let new_hostname = args[0];
        match fs::write_file("/cfg/hostname", new_hostname.as_bytes()) {
            Ok(()) => {
                console.print_colored("Hostname set to: ", COLOR_GREEN);
                console.print(new_hostname);
                console.put_char('\n');
            }
            Err(e) => {
                console.print_colored("hostname: error setting hostname: ", crate::framebuffer::COLOR_RED);
                console.print(&format!("{:?}", e));
                console.put_char('\n');
            }
        }
    }
    CmdResult::Ok
}
