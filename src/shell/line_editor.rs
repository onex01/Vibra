use crate::keyboard::{self, Key};
use crate::framebuffer::Console;
use crate::commands;
use crate::fs::{self, FileType};

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
        }
    }

    pub fn read_line(&mut self, console: &mut Console, prompt_len: usize) -> &str {
        self.len = 0;
        self.cursor = 0;
        self.history_idx = self.history_count;

        loop {
            if let Some(key) = keyboard::poll_key() {
                match key {
                    Key::Enter => {
                        console.put_char('\n');
                        self.save_to_history();
                        return self.as_str();
                    }
                    Key::Backspace => {
                        if self.cursor > 0 {
                            for i in self.cursor..self.len {
                                self.buffer[i-1] = self.buffer[i];
                            }
                            self.len -= 1;
                            self.cursor -= 1;
                            self.redraw(console, prompt_len);
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
                            self.redraw(console, prompt_len);
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
                            self.redraw(console, prompt_len);
                        }
                    }
                    Key::Tab => {
                        self.tab_complete(console, prompt_len);
                    }
                    Key::Char(ch) => {
                        if self.len < MAX_LINE - 1 {
                            for i in (self.cursor..self.len).rev() {
                                self.buffer[i+1] = self.buffer[i];
                            }
                            self.buffer[self.cursor] = ch as u8;
                            self.len += 1;
                            self.cursor += 1;
                            self.redraw(console, prompt_len);
                        }
                    }
                    _ => {}
                }
            }
            core::hint::spin_loop();
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.len]).unwrap_or("")
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

    fn redraw(&self, console: &mut Console, prompt_len: usize) {
        // 1. Возвращаемся в начало ввода (после prompt)
        for _ in 0..(self.cursor + prompt_len) {
            console.put_char('\x08');
        }
        
        // 2. Стираем текущий текст (для этого идем до конца строки)
        for _ in 0..self.len {
            console.put_char(' ');
        }
        
        // 3. Возвращаемся в начало ввода
        for _ in 0..(self.cursor + prompt_len) {
            console.put_char('\x08');
        }
        
        // 4. Рисуем текст
        if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
            console.print(s);
        }
        
        // 5. Возвращаем курсор на правильную позицию
        for _ in self.cursor..self.len {
            console.put_char('\x08');
        }
    }

    fn tab_complete(&mut self, console: &mut Console, prompt_len: usize) {
        let line = self.as_str();
        let trimmed = line.trim();

        if !trimmed.contains(' ') {
            let mut matches: [&str; 32] = [""; 32];
            let mut n_matches = 0usize;

            for name in commands::command_names() {
                if name.starts_with(trimmed) && n_matches < 32 {
                    matches[n_matches] = name;
                    n_matches += 1;
                }
            }

            if n_matches == 1 {
                let full = matches[0];
                let to_append = &full[trimmed.len()..];
                self.append_str(to_append);
                self.append_str(" ");
                self.redraw(console, prompt_len);
            } else if n_matches > 1 {
                console.put_char('\n');
                for i in 0..n_matches {
                    console.print("  ");
                    console.print(matches[i]);
                }
                console.put_char('\n');
                console.print("vibra> ");
                if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                    console.print(s);
                }
            }
        } else {
            let parts: [&str; 16] = {
                let mut arr = [""; 16];
                let mut n = 0;
                for p in trimmed.split_whitespace() {
                    if n < 16 { arr[n] = p; n += 1; }
                }
                arr
            };

            let last_part = parts.iter().rev().find(|s| !s.is_empty()).copied().unwrap_or("");
            let mut matches: [&str; 32] = [""; 32];
            let mut n_matches = 0usize;

            for entry in fs::list_entries() {
                if entry.name().starts_with(last_part) && n_matches < 32 {
                    matches[n_matches] = entry.name();
                    n_matches += 1;
                }
            }

            if n_matches == 1 {
                let full = matches[0];
                let to_append = &full[last_part.len()..];
                self.append_str(to_append);
                if matches.iter().take(n_matches).any(|n| {
                    fs::list_entries().any(|e| e.name() == *n && e.metadata.file_type == FileType::Directory)
                }) {
                    self.append_str("/");
                } else {
                    self.append_str(" ");
                }
                self.redraw(console, prompt_len);
            } else if n_matches > 1 {
                console.put_char('\n');
                for i in 0..n_matches {
                    console.print("  ");
                    console.print(matches[i]);
                }
                console.put_char('\n');
                console.print("vibra> ");
                if let Ok(s) = core::str::from_utf8(&self.buffer[..self.len]) {
                    console.print(s);
                }
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