use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let editor = crate::LINE_EDITOR.lock();
    let history_count = editor.history_count();

    if history_count == 0 {
        console.print("История команд пуста\n");
        return CmdResult::Ok;
    }

    for i in 0..history_count {
        if let Some(entry) = editor.history_entry(i) {
            // Номер записи (начинается с 1)
            let num = i + 1;
            if num < 10 {
                console.print("  ");
            } else if num < 100 {
                console.print(" ");
            }
            console.print_num(num);
            console.print("  ");
            console.print(entry);
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}
