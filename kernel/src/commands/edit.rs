use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_RED, COLOR_GREEN, COLOR_CYAN};
use crate::fs;
use crate::keyboard::{self, Key};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(name) = args.first() else {
        console.print_colored("Usage: edit <filename>\n", COLOR_RED);
        return CmdResult::Ok;
    };

    if fs::read_file(name).is_err() {
        if let Err(e) = fs::create_file(name) {
            console.print_colored("Error creating file: ", COLOR_RED);
            console.print(&format!("{}", e)); console.put_char('\n');
            return CmdResult::Ok;
        }
    }

    let mut buffer = [0u8; 2048];
    let mut len = 0usize;

    if let Ok(data) = fs::read_file(name) {
        len = data.len().min(buffer.len());
        buffer[..len].copy_from_slice(&data[..len]);
    }

    console.print_colored("Editing: ", COLOR_YELLOW);
    console.print(name);
    console.print_colored(" (ESC to save & exit)\n", COLOR_YELLOW);
    console.print_colored("---\n", COLOR_CYAN);

    if len > 0 {
        if let Ok(text) = core::str::from_utf8(&buffer[..len]) {
            console.print(text);
            console.put_char('\n');
        }
    }

    let mut cursor_pos = len;
    console.print("> ");

    loop {
        if let Some(key) = keyboard::poll_key() {
            match key {
                Key::Char('\x1B') => {
                    // ESC — выходим из цикла
                    break;
                }
                Key::Enter => {
                    if len < buffer.len() {
                        buffer[len] = b'\n';
                        len += 1;
                        cursor_pos = len;
                        console.put_char('\n');
                        console.print("> ");
                    }
                }
                Key::Backspace => {
                    if cursor_pos > 0 {
                        for i in cursor_pos..len { buffer[i-1] = buffer[i]; }
                        len -= 1;
                        cursor_pos -= 1;
                        redraw_editor(console, &buffer[..len], cursor_pos);
                    }
                }
                Key::Char(ch) => {
                    if len < buffer.len() {
                        for i in (cursor_pos..len).rev() { buffer[i+1] = buffer[i]; }
                        buffer[cursor_pos] = ch as u8;
                        len += 1;
                        cursor_pos += 1;
                        redraw_editor(console, &buffer[..len], cursor_pos);
                    }
                }
                _ => {}
            }
        }
        core::hint::spin_loop();
    }

    // Сохраняем (даже если пустой файл)
    match fs::write_file(name, &buffer[..len]) {
        Ok(_) => {
            console.print_colored("\nSaved: ", COLOR_GREEN);
            console.print(name);
            console.print(" (");
            console.print_num(len);
            console.print(" bytes)\n");
        }
        Err(e) => {
            console.print_colored("\nError saving: ", COLOR_RED);
            console.print(&format!("{}", e)); console.put_char('\n');
        }
    }
    CmdResult::Ok
}

fn redraw_editor(console: &mut Console, data: &[u8], _cursor_pos: usize) {
    // Находим последнюю строку
    let last_line = if let Ok(text) = core::str::from_utf8(data) {
        text.rsplit('\n').next().unwrap_or("")
    } else {
        ""
    };

    // Возвращаем каретку к началу текущей строки (после "> ")
    let line_len = last_line.len();
    for _ in 0..line_len { console.put_char('\x08'); }
    
    // Стираем строку
    for _ in 0..line_len { console.put_char(' '); }
    for _ in 0..line_len { console.put_char('\x08'); }
    
    // Рисуем заново
    console.print(last_line);
}