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
    prompt_buf: [u8; 128],
    prompt_len: usize,
    prompt_color: u32,
    // Что реально нарисовано в строке ввода. Нужно для корректной очистки:
    // нельзя опираться на новый len/cursor после загрузки истории.
    screen_len: usize,
    screen_cursor: usize,
    /// Позиция начала ввода (сразу после приглашения) и её длина.
    ///
    /// Раньше перерисовка стирала текст последовательностью `\x08`, а он не
    /// переходит на предыдущую строку при переносе: длинная команда
    /// оставляла в приглашении обрывки предыдущего текста. Теперь позиция
    /// известна явно, и стирание идёт по координатам.
    input_col: usize,
    input_row: usize,
    prompt_len_cells: usize,
    tab_index: i32,
    tab_match_count: usize,
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
            prompt_color: crate::framebuffer::COLOR_VIBRA_PROMPT,
            screen_len: 0,
            screen_cursor: 0,
            input_col: 0,
            input_row: 0,
            prompt_len_cells: 0,
            tab_index: -1,
            tab_match_count: 0,
        }
    }

    pub fn read_line(&mut self, console: &mut Console, prompt_len: usize) -> &str {
        self.len = 0;
        self.cursor = 0;
        self.history_idx = self.history_count;
        self.prompt_len = prompt_len;
        self.screen_len = 0;
        self.screen_cursor = 0;
        self.prompt_len_cells = prompt_len;
        // Запоминаем позицию НАЧАЛА ПРИГЛАШЕНИЯ (курсор уже сдвинут за него):
        // от неё считаются и стирание, и позиция курсора ввода.
        let (col, row) = console.cursor_position();
        let cols = console.fb_cols().max(1);
        let start = col.saturating_sub(self.prompt_len_cells);
        self.input_row = row - (self.prompt_len_cells.saturating_sub(col)) / cols;
        self.input_col = start % cols;
        // Новая сессия ввода: состояние курсора начинается с чистого листа.
        console.reset_cursor_blink();
        // Стираем остаток строки после приглашения: в ней мог остаться текст
        // предыдущей команды, если та закончила вывод без перевода строки.
        // Курсор уже стоит сразу после приглашения — `col` и есть начало ввода.
        console.clear_range(col, row, console.fb_cols().saturating_sub(1), row);
        let mut last_heartbeat_ms = crate::timer::uptime_ms();
        let mut last_log_flush_ms = last_heartbeat_ms;
        let mut last_hotplug_ms = last_heartbeat_ms;
        // Интервал записи лога. Если носитель отвечает медленно (USB-
        // накопитель), запись растягивается на сотни миллисекунд и
        // превращается в «залипание» клавиатуры: ввод ждёт её в этом же
        // цикле. Тогда интервал растёт, вместо того чтобы тормозить ввод.
        let mut log_flush_interval_ms: u64 = 1_000;
        let mut log_slow_reported = false;

        loop {
            let now_ms = crate::timer::uptime_ms();
            // Hotplug опрашивается из цикла ввода: только здесь ядро
            // проводит время в ожидании, прерывание PORTSC само по себе
            // устройство не перечисляет.
            if now_ms.saturating_sub(last_hotplug_ms) >= 500 {
                last_hotplug_ms = now_ms;
                let (added, removed) = crate::drivers::usb::poll_hotplug();
                if added > 0 || removed > 0 {
                    console.put_char('\n');
                    console.print_colored(
                        "[USB] port change: ",
                        crate::framebuffer::COLOR_YELLOW,
                    );
                    if added > 0 {
                        console.print("connected, ");
                    }
                    if removed > 0 {
                        console.print("disconnected, ");
                    }
                    console.print("run 'disks' to rescan\n");
                    // Приглашение и набранный текст возвращаем на место.
                    // Раньше строка ввода пропадала после сообщения и shell
                    // выглядел повешенным, хотя продолжал читать ввод.
                    self.reprint_input(console);
                }
            }

            // В простое экран догоняет тень: во время массового вывода
            // перерисовка откладывалась, иначе прокрутка стоила бы десятки
            // миллисекунд на строку.
            console.flush_if_due(false);

            let next_key = crate::serial::poll_key().or_else(keyboard::poll_key);
            if let Some(key) = next_key {
                match key {
                    Key::Enter => {
                        console.put_char('\n');
                        self.screen_len = 0;
                        self.screen_cursor = 0;
                        self.save_to_history();
                        return self.as_str();
                    }
                    Key::Backspace => {
                        if self.cursor > 0 {
                            for i in self.cursor..self.len {
                                self.buffer[i - 1] = self.buffer[i];
                            }
                            self.len -= 1;
                            self.cursor -= 1;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::Left => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::Right => {
                        if self.cursor < self.len {
                            self.cursor += 1;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::Home => {
                        if self.cursor != 0 {
                            self.cursor = 0;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::End => {
                        if self.cursor != self.len {
                            self.cursor = self.len;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    // Delete — удалить символ под курсором (как в любом
                    // редакторе строки). Раньше клавиша игнорировалась.
                    Key::Delete => {
                        if self.cursor < self.len {
                            for i in self.cursor + 1..self.len {
                                self.buffer[i - 1] = self.buffer[i];
                            }
                            self.len -= 1;
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    // Insert в строке ввода не нужен: режим вставки всегда.
                    Key::Insert => {}
                    // Ctrl+влево/вправо — переход по словам.
                    Key::WordLeft => {
                        while self.cursor > 0
                            && self.buffer[self.cursor - 1] == b' '
                        {
                            self.cursor -= 1;
                        }
                        while self.cursor > 0
                            && self.buffer[self.cursor - 1] != b' '
                        {
                            self.cursor -= 1;
                        }
                        self.resync_origin(console);
                        self.redraw_line(console);
                    }
                    Key::WordRight => {
                        while self.cursor < self.len && self.buffer[self.cursor] != b' ' {
                            self.cursor += 1;
                        }
                        while self.cursor < self.len && self.buffer[self.cursor] == b' ' {
                            self.cursor += 1;
                        }
                        self.resync_origin(console);
                        self.redraw_line(console);
                    }
                    // PageUp/PageDown листают историю: в строке ввода
                    // «страницы» — это записи истории.
                    Key::PageUp => {
                        if self.history_count > 0 && self.history_idx > 0 {
                            self.history_idx -= 1;
                            self.load_from_history(self.history_idx);
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::PageDown => {
                        if self.history_idx < self.history_count {
                            self.history_idx += 1;
                            if self.history_idx == self.history_count {
                                self.len = 0;
                                self.cursor = 0;
                            } else {
                                self.load_from_history(self.history_idx);
                            }
                            self.resync_origin(console);
                            self.redraw_line(console);
                        }
                    }
                    Key::Up => {
                        if self.history_count > 0 && self.history_idx > 0 {
                            self.history_idx -= 1;
                            self.load_from_history(self.history_idx);
                            self.redraw_line(console);
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
                            self.redraw_line(console);
                        }
                    }
                    Key::Tab => {
                        self.tab_complete(console);
                    }
                    Key::Char(ch) => {
                        self.resync_origin(console);
                        if ch != '\x01' && ch != '\x05' && ch != '\x0B'
                            && ch != '\x0C' && ch != '\x15' && ch != '\x1A'
                            && ch != '\x03'
                        {
                            self.tab_index = -1;
                        }

                        match ch {
                            '\x01' => { // Ctrl+A
                                self.cursor = 0;
                                self.redraw_line(console);
                            }
                            '\x05' => { // Ctrl+E
                                self.cursor = self.len;
                                self.redraw_line(console);
                            }
                            '\x0B' => { // Ctrl+K
                                self.len = self.cursor;
                                self.redraw_line(console);
                            }
                            '\x15' => { // Ctrl+U
                                if self.cursor > 0 {
                                    for i in self.cursor..self.len {
                                        self.buffer[i - self.cursor] = self.buffer[i];
                                    }
                                    self.len -= self.cursor;
                                    self.cursor = 0;
                                    self.redraw_line(console);
                                }
                            }
                            '\x0C' => { // Ctrl+L
                                console.clear();
                                self.print_prompt(console);
                                self.screen_len = 0;
                                self.screen_cursor = 0;
                                self.redraw_line(console);
                            }
                            '\x03' => { // Ctrl+C
                                crate::request_cancel();
                                self.len = 0;
                                self.cursor = 0;
                                console.print_colored("^C\n", crate::framebuffer::COLOR_RED);
                                self.screen_len = 0;
                                self.screen_cursor = 0;
                                return "";
                            }
                            '\x1A' => { // Ctrl+Z
                                crate::request_cancel();
                                self.len = 0;
                                self.cursor = 0;
                                console.put_char('\n');
                                self.screen_len = 0;
                                self.screen_cursor = 0;
                                return "";
                            }
                            _ => {
                                if self.len < MAX_LINE - 1 {
                                    for i in (self.cursor..self.len).rev() {
                                        self.buffer[i + 1] = self.buffer[i];
                                    }
                                    self.buffer[self.cursor] = ch as u8;
                                    self.len += 1;
                                    self.cursor += 1;
                                    self.redraw_line(console);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            if next_key.is_none() {
                let now_ms = crate::timer::uptime_ms();
                // Лог пишется только когда ввод простаивает, и с замером:
                // медленный носитель не должен задерживать клавиатуру.
                if now_ms.wrapping_sub(last_log_flush_ms) >= log_flush_interval_ms {
                    let started = crate::timer::uptime_ms();
                    crate::boot_log::checkpoint();
                    let took = crate::timer::uptime_ms().saturating_sub(started);
                    last_log_flush_ms = crate::timer::uptime_ms();
                    if took > 100 {
                        log_flush_interval_ms = (log_flush_interval_ms * 2).min(15_000);
                        if !log_slow_reported {
                            log_slow_reported = true;
                            crate::println!(
                                "[LOG] checkpoint took {} ms: flash writes are slow, backing off",
                                took
                            );
                        }
                    } else if log_flush_interval_ms > 1_000 && took < 20 {
                        log_flush_interval_ms = 1_000;
                    }
                }
                if now_ms.wrapping_sub(last_heartbeat_ms) >= 5_000 {
                    crate::serial::debugcon_println(format_args!(
                        "[HEARTBEAT] uptime={}s ticks={} clock={}",
                        now_ms / 1000,
                        crate::interrupts::idt::ticks(),
                        crate::timer::current_source().name()
                    ));
                    last_heartbeat_ms = now_ms;
                }

                console.update_cursor_blink();
                crate::timer::idle_wait();
            }
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.len]).unwrap_or("")
    }

    pub fn set_prompt(&mut self, prompt: &str) {
        let bytes = prompt.as_bytes();
        let len = bytes.len().min(self.prompt_buf.len());
        self.prompt_buf[..len].copy_from_slice(&bytes[..len]);
        self.prompt_len = len;
    }

    pub fn set_prompt_color(&mut self, color: u32) {
        self.prompt_color = color;
    }

    fn print_prompt(&self, console: &mut Console) {
        if let Ok(p) = core::str::from_utf8(&self.prompt_buf[..self.prompt_len]) {
            console.print_colored(p, self.prompt_color);
        }
    }

    /// Перерисовать только строку ввода, не трогая prompt.
    ///
    /// Опирается на `screen_len`/`screen_cursor` — то, что реально на экране.
    /// Иначе после загрузки истории новый `len` больше старого, и backspace
    /// уезжает в prompt — ровно та проблема, из-за которой пропадал `vibra:/>`.
    /// Убедиться, что экран актуален и позиция начала ввода ещё верна.
    ///
    /// После большого вывода отложенная перерисовка сдвигает экран, и
    /// сохранённые координаты приглашения устаревают: стирание начинает
    /// бить в другое место, оставляя в строке ввода обрывки предыдущего
    /// текста. Здесь мы догоняем экран и при необходимости пере-закрепляем
    /// начало ввода на текущую позицию курсора.
    fn resync_origin(&mut self, console: &mut Console) {
        // Именно принудительно: троттлинг в 40 мс оставлял экран на строку
        // позади, и все вычисления ниже шли по устаревшим координатам.
        console.flush_if_due(true);
        let (col, row) = console.cursor_position();

        // Курсор редактора всегда лежит на строке ввода; если он оказался
        // выше или левее сохранённого начала, значит экран сдвинулся.
        let expected_row = self.input_row;
        let in_input_area = row == expected_row
            || (row == expected_row + 1 && col < self.input_col);
        if !in_input_area || col < self.input_col {
            // Пере-закрепляем: считаем, что ввод начинается там, где сейчас
            // стоит курсор минус уже набранный текст.
            let consumed = self.screen_len;
            let total = consumed as isize;
            let cols = console.fb_cols().max(1) as isize;
            let mut new_col = col as isize - total;
            let mut new_row = row;
            while new_col < 0 {
                if new_row == 0 {
                    new_col = 0;
                    break;
                }
                new_row -= 1;
                new_col += cols;
            }
            self.input_col = new_col as usize;
            self.input_row = new_row;
        }
    }

    /// Вернуть приглашение и набранный текст после служебного сообщения.
    ///
    /// Служебные строки (подключение USB, предупреждения) печатаются посреди
    /// ввода, поэтому после них нужно заново нарисовать приглашение и
    /// восстановить начало строки ввода — иначе shell выглядит зависшим.
    fn reprint_input(&mut self, console: &mut Console) {
        self.print_prompt(console);
        let (col, row) = console.cursor_position();
        let cols = console.fb_cols().max(1);
        // Те же формулы, что и при входе в read_line: курсор стоит сразу
        // после приглашения, начало ввода — на `prompt_len_cells` раньше.
        let start = col.saturating_sub(self.prompt_len_cells);
        self.input_row = row - (self.prompt_len_cells.saturating_sub(col)) / cols;
        self.input_col = start % cols;
        self.screen_len = 0;
        self.screen_cursor = 0;
        self.redraw_line(console);
    }

    fn redraw_line(&mut self, console: &mut Console) {
        // Стираем ровно то, что было напечатано: от начала ввода до конца
        // прошлого текста. Раньше конец брался из текущей позиции курсора, а
        // она уже сдвинута — стиралось меньше, и в строке оставались обрывки.
        // Начало ввода — это КОНЕЦ приглашения: стирать от `input_col`
        // нельзя, там ещё стоит сам prompt, и первая буква команды съедалась.
        let cols = console.fb_cols().max(1);
        let begin = self.input_col + self.prompt_len_cells;
        let begin_row = self.input_row + begin / cols;
        let begin_col = begin % cols;
        let end = begin + self.screen_len.max(self.len);
        console.clear_range(
            begin_col,
            begin_row,
            end % cols,
            begin_row + end / cols,
        );

        if let Ok(text) = core::str::from_utf8(&self.buffer[..self.len]) {
            console.print(text);
        }

        // Ставим курсор туда, где он должен быть по логике редактора.
        // Считаем от реального начала ввода (`input_col`), а не от длины
        // приглашения: если приглашение переносилось, это разные вещи.
        let cols = console.fb_cols().max(1);
        let cells = self.input_col + self.prompt_len_cells + self.cursor;
        let row = self.input_row + cells / cols;
        let col = cells % cols;
        console.set_cursor_position(col, row);

        // Курсор рисуем сразу: иначе он появляется только по таймеру
        // мигания, и между нажатиями его не видно.
        console.draw_cursor();

        self.screen_len = self.len;
        self.screen_cursor = self.cursor;
    }

    fn save_to_history(&mut self) {
        if self.len == 0 {
            return;
        }
        if self.history_count < HISTORY_SIZE {
            self.history[self.history_count][..self.len].copy_from_slice(&self.buffer[..self.len]);
            self.history_lens[self.history_count] = self.len;
            self.history_count += 1;
        } else {
            for i in 0..HISTORY_SIZE - 1 {
                self.history[i] = self.history[i + 1];
                self.history_lens[i] = self.history_lens[i + 1];
            }
            self.history[HISTORY_SIZE - 1][..self.len].copy_from_slice(&self.buffer[..self.len]);
            self.history_lens[HISTORY_SIZE - 1] = self.len;
        }
        self.history_idx = self.history_count;
    }

    fn load_from_history(&mut self, idx: usize) {
        let src_len = self.history_lens[idx];
        self.buffer[..src_len].copy_from_slice(&self.history[idx][..src_len]);
        self.len = src_len;
        self.cursor = src_len;
    }

    fn tab_complete(&mut self, console: &mut Console) {
        let line = match core::str::from_utf8(&self.buffer[..self.len]) {
            Ok(s) => String::from(s),
            Err(_) => return,
        };

        if line.contains(' ') || line.starts_with('/') {
            self.complete_path(console);
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        let mut matches: [&str; 32] = [""; 32];
        let mut n_matches = 0usize;
        for name in commands::command_names() {
            if name.starts_with(trimmed) && n_matches < 32 {
                matches[n_matches] = name;
                n_matches += 1;
            }
        }
        if n_matches == 0 {
            return;
        }

        self.tab_match_count = n_matches;
        if self.tab_index < 0 || self.tab_index >= n_matches as i32 {
            self.tab_index = 0;
        } else {
            self.tab_index = (self.tab_index + 1) % n_matches as i32;
        }

        self.len = 0;
        self.cursor = 0;
        self.append_str(matches[self.tab_index as usize]);
        self.append_str(" ");
        self.redraw_line(console);
    }

    /// Дополняет последнее слово строки как путь, сохраняя предыдущие аргументы.
    fn complete_path(&mut self, console: &mut Console) {
        let line = match core::str::from_utf8(&self.buffer[..self.len]) {
            Ok(s) => String::from(s),
            Err(_) => return,
        };
        let line = line.as_str();

        let split = line.rfind(' ').map(|i| i + 1).unwrap_or(0);
        let before = &line[..split];
        let word = &line[split..];
        if word.is_empty() {
            return;
        }

        let current_dir = crate::fs::get_current_dir();
        let (search_dir, display_prefix, name_prefix) = if let Some(slash) = word.rfind('/') {
            let dir_part = &word[..=slash];
            let name = &word[slash + 1..];
            let search = if dir_part.starts_with('/') {
                String::from(dir_part)
            } else if current_dir == "/" {
                format!("/{}", dir_part)
            } else {
                format!("{}/{}", current_dir, dir_part)
            };
            (search, String::from(dir_part), String::from(name))
        } else {
            (current_dir.clone(), String::new(), String::from(word))
        };

        let entries = crate::fs::list_dir(&search_dir);
        let mut matches: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        for entry in &entries {
            if !entry.name.starts_with(&name_prefix) {
                continue;
            }
            let mut full = display_prefix.clone();
            full.push_str(&entry.name);
            if entry.file_type == crate::fs::FileType::Directory {
                full.push('/');
            }
            matches.push(full);
        }
        if matches.is_empty() {
            return;
        }

        self.tab_match_count = matches.len();
        if self.tab_index < 0 || self.tab_index >= matches.len() as i32 {
            self.tab_index = 0;
        } else {
            self.tab_index = (self.tab_index + 1) % matches.len() as i32;
        }

        self.len = 0;
        self.cursor = 0;
        self.append_str(before);
        self.append_str(&matches[self.tab_index as usize]);
        self.redraw_line(console);
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

    pub fn history_count(&self) -> usize {
        self.history_count
    }

    pub fn history_entry(&self, idx: usize) -> Option<&str> {
        if idx >= self.history_count {
            return None;
        }
        let len = self.history_lens[idx];
        core::str::from_utf8(&self.history[idx][..len]).ok()
    }
}