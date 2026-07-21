use crate::keyboard::{self, Key};
use crate::framebuffer::Console;
use crate::commands;
use alloc::string::String;
use alloc::format;

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 16;

pub struct LineEditor {
    buffer: [u8; MAX_LINE],
    len: usize,
    cursor: usize,
    history: [[u8; MAX_LINE]; HISTORY_SIZE],
    history_lens: [usize; HISTORY_SIZE],
    history_count: usize,
    history_idx: usize,
    prompt_buf: [u8; 128], // буфер для prompt строки
    prompt_len: usize,
}

impl LineEditor {
    pub const fn new() -> Self {
        LineEditor {
            buffer: [0; MAX_LINE],
            len: 0,
            cursor: 0,
            history: [[0; MAX_LINE]; HISTORY_SIZE],
            history_lens: [0; HISTORY_SIZE],
            history_count: 0,
            history_idx: 0,
            prompt_buf: [0; 128],
            prompt_len: 0,
        }
    }

    pub fn read_line(&mut self, console: &mut Console, prompt_len: usize) -> &str {
        self.len = 0;
        self.cursor = 0;
        self.history_idx = self.history_count;
        self.prompt_len = prompt_len;
        loop {
            let next_key = keyboard::poll_key().or_else(crate::serial::poll_key);
            if let Some(key) = next_key {
                match key {
                    Key::Enter => {
                        console.put_char('\n');
                        self.save_to_history();
                        return self.as_str();
                    }
                    Key::Backspace => {
                        if self.cursor > 0 {
                            // Сдвигаем всё влево
                            for i in self.cursor..self.len {
                                self.buffer[i-1] = self.buffer[i];
                            }
                            self.len -= 1;
                            self.cursor -= 1;
                            
                            // Терминальный backspace: удаляем символ
                            console.put_char('\x08');
                            console.put_char(' ');
                            console.put_char('\x08');
                            
                            // Перепечатываем остаток строки
                            for i in self.cursor..self.len {
                                console.put_char(self.buffer[i] as char);
                            }
                            // Возвращаем курсор на место
                            for _ in self.cursor..self.len {
                                console.put_char('\x08');
                            }
                        }
                    }
                    Key::Left => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            console.put_char('\x08');
                        }
                    }
                    Key::Right => {
                        if self.cursor < self.len {
                            let ch = self.buffer[self.cursor] as char;
                            console.put_char(ch);
                            self.cursor += 1;
                        }
                    }
                    Key::Up => {
                        if self.history_count > 0 && self.history_idx > 0 {
                            self.history_idx -= 1;
                            self.load_from_history(self.history_idx);
                            // Перепечатываем всю строку
                            self.reprint_line(console);
                        }
                    }
                    Key::Down => {
                        if self.history_idx < self.history_count {
                            self.history_idx += 1;
                            if self.history_idx == self.history_count {
                                self.len = 0;
                                self.cursor = 0;
                            } else {
                                self.load_from_history(self.history_idx);
                            }
                            self.reprint_line(console);
                        }
                    }
                    Key::Tab => {
                        self.tab_complete(console);
                    }
                    Key::Char(ch) => {
                        if self.len < MAX_LINE - 1 {
                            // Вставляем символ
                            for i in (self.cursor..self.len).rev() {
                                self.buffer[i+1] = self.buffer[i];
                            }
                            self.buffer[self.cursor] = ch as u8;
                            self.len += 1;
                            self.cursor += 1;
                            
                            // Просто печатаем символ
                            console.put_char(ch);
                            
                            // Перепечатываем остаток строки
                            for i in self.cursor..self.len {
                                console.put_char(self.buffer[i] as char);
                            }
                            // Возвращаем курсор
                            for _ in self.cursor..self.len {
                                console.put_char('\x08');
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Если ввода не было, спим до следующего прерывания вместо
            // busy-loop. После принятого serial-байта не спим: так быстро
            // разгружаем 16-байтный FIFO UART и не теряем длинные команды.
            if next_key.is_none() {
                crate::interrupts::wait();
            }
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.len]).unwrap_or("")
    }

    /// Установить prompt строку (для tab completion)
    pub fn set_prompt(&mut self, prompt: &str) {
        let bytes = prompt.as_bytes();
        let len = bytes.len().min(self.prompt_buf.len());
        self.prompt_buf[..len].copy_from_slice(&bytes[..len]);
        self.prompt_len = len;
    }

    fn save_to_history(&mut self) {
        if self.len == 0 { return; }
        if self.history_count < HISTORY_SIZE {
            self.history[self.history_count][..self.len].copy_from_slice(&self.buffer[..self.len]);
            self.history_lens[self.history_count] = self.len;
            self.history_count += 1;
        } else {
            for i in 0..HISTORY_SIZE-1 {
                self.history[i] = self.history[i+1];
                self.history_lens[i] = self.history_lens[i+1];
            }
            self.history[HISTORY_SIZE-1][..self.len].copy_from_slice(&self.buffer[..self.len]);
            self.history_lens[HISTORY_SIZE-1] = self.len;
        }
        self.history_idx = self.history_count;
    }

    fn load_from_history(&mut self, idx: usize) {
        let src_len = self.history_lens[idx];
        self.buffer[..src_len].copy_from_slice(&self.history[idx][..src_len]);
        self.len = src_len;
        self.cursor = src_len;
    }

    // Перепечатывает текущую строку (для history)
    fn reprint_line(&self, console: &mut Console) {
        // Возвращаемся в начало строки
        for _ in 0..self.len {
            console.put_char('\x08');
        }
        // Стираем
        for _ in 0..self.len {
            console.put_char(' ');
        }
        // Возвращаемся
        for _ in 0..self.len {
            console.put_char('\x08');
        }
        // Рисуем
        if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
            console.print(s);
        }
    }

    fn tab_complete(&mut self, console: &mut Console) {
        let line = self.as_str();
        let trimmed = line.trim();

        if !trimmed.contains(' ') {
            // Автодополнение команд
            let mut matches: [&str; 32] = [""; 32];
            let mut n_matches = 0usize;

            for name in commands::command_names() {
                if name.starts_with(trimmed) && n_matches < 32 {
                    matches[n_matches] = name;
                    n_matches += 1;
                }
            }

            if n_matches == 1 {
                // Один матч — дополняем
                let full = matches[0];
                // Очищаем текущий ввод (учитывая длину prompt)
                let total = self.prompt_len + self.len;
                for _ in 0..total {
                    console.put_char('\x08');
                }
                for _ in 0..total {
                    console.put_char(' ');
                }
                for _ in 0..total {
                    console.put_char('\x08');
                }
                
                // Записываем полную команду
                self.len = 0;
                self.cursor = 0;
                self.append_str(full);
                self.append_str(" ");
                
                // Печатаем prompt + команду
                if let Ok(prompt_str) = core::str::from_utf8(&self.prompt_buf[..self.prompt_len]) {
                    console.print(prompt_str);
                }
                if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                    console.print(s);
                }
            } else if n_matches > 1 {
                // Несколько матчей — показываем список
                console.put_char('\n');
                for i in 0..n_matches {
                    console.print("  ");
                    console.print(matches[i]);
                }
                console.put_char('\n');
                // Печатаем prompt + текущий буфер
                if let Ok(prompt_str) = core::str::from_utf8(&self.prompt_buf[..self.prompt_len]) {
                    console.print(prompt_str);
                }
                if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                    console.print(s);
                }
            }
        } else {
            // Автодополнение путей файлов/каталогов
            self.complete_path(console);
        }
    }

    /// Автодополнение пути к файлу/каталогу
    fn complete_path(&mut self, console: &mut Console) {
        // Копируем данные в стековый буфер чтобы избежать borrow conflict
        let mut line_buf = [0u8; 256];
        let line_len = self.len;
        let copy_len = line_len.min(255);
        line_buf[..copy_len].copy_from_slice(&self.buffer[..copy_len]);
        let line = core::str::from_utf8(&line_buf[..copy_len]).unwrap_or("");
        let parts: alloc::vec::Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return;
        }

        let partial = parts[1];
        let current_dir = crate::fs::get_current_dir();

        // Определяем директорию для поиска
        let (search_dir, prefix) = if partial.is_empty() || !partial.contains('/') {
            (current_dir.clone(), String::new())
        } else {
            if let Some(last_slash) = partial.rfind('/') {
                let dir_part = &partial[..last_slash];
                let name_part = &partial[last_slash + 1..];
                let full_dir = if dir_part.starts_with('/') {
                    String::from(dir_part)
                } else if current_dir == "/" {
                    format!("/{}", dir_part)
                } else {
                    format!("{}/{}", current_dir, dir_part)
                };
                (full_dir, String::from(name_part))
            } else {
                (current_dir.clone(), String::from(partial))
            }
        };

        // Ищем совпадения
        let entries = crate::fs::list_dir(&search_dir);
        let mut matches: alloc::vec::Vec<String> = alloc::vec::Vec::new();

        for entry in &entries {
            if entry.name.starts_with(&prefix) {
                let mut full = String::new();
                if !search_dir.ends_with('/') {
                    full.push_str(&search_dir);
                }
                full.push_str(&entry.name);
                if entry.file_type == crate::fs::FileType::Directory {
                    full.push('/');
                }
                matches.push(full);
            }
        }

        if matches.len() == 1 {
            // Один матч — дополняем
            let completion = &matches[0];
            let total = self.prompt_len + self.len;
            for _ in 0..total {
                console.put_char('\x08');
            }
            for _ in 0..total {
                console.put_char(' ');
            }
            for _ in 0..total {
                console.put_char('\x08');
            }

            // Очищаем буфер и записываем новую строку
            self.len = 0;
            self.cursor = 0;
            self.append_str(parts[0]);
            self.append_str(" ");
            self.append_str(completion);

            // Печатаем prompt + строку
            if let Ok(p) = core::str::from_utf8(&self.prompt_buf[..self.prompt_len]) {
                console.print(p);
            }
            if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                console.print(s);
            }
        } else if matches.len() > 1 {
            // Несколько совпадений — показываем список
            console.put_char('\n');
            for m in &matches {
                console.print("  ");
                console.print(m);
            }
            console.put_char('\n');
            if let Ok(p) = core::str::from_utf8(&self.prompt_buf[..self.prompt_len]) {
                console.print(p);
            }
            if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                console.print(s);
            }
        }
    }

    fn append_str(&mut self, s: &str) {
        for b in s.bytes() {
            if self.len < MAX_LINE - 1 {
                self.buffer[self.len] = b;
                self.len += 1;
                self.cursor += 1;
            }
        }
    }
}
