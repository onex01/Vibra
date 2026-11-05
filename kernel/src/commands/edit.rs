// edit — полноэкранный текстовый редактор в духе nano.
//
// Раньше это была заготовка: файл целиком (не больше 2 КиБ) печатался в
// поток, а «редактирование» сводилось к стиранию последней строки
// backspace-последовательностями. Ни курсора, ни прокрутки, ни сохранения
// по требованию не было.
//
// Здесь: буфер — обычный `Vec<u8>` с разделителями '\n', курсор — байтовое
// смещение. Строки не хранятся отдельно, поэтому вставка '\n' и склейка
// строк (Backspace на начале строки) получаются сами собой.
//
// Управление (как в nano):
//   стрелки, Home/End      — движение (Home/End = Ctrl+A / Ctrl+E)
//   Ctrl+Y / Ctrl+V        — страница вверх / вниз
//   Ctrl+O                 — сохранить
//   Ctrl+X                 — выход (если есть изменения — предупреждает)
//   Ctrl+K                 — удалить строку
//   Ctrl+D                 — удалить символ под курсором
//   Tab                    — отступ (4 пробела)
//
// Что важно для железа: экран перерисовывается только при изменении
// состояния, а не по таймеру — иначе на uncached-фреймбуфере редактор
// съедал бы всё время.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_DARK_GRAY, COLOR_RED, COLOR_YELLOW};
use crate::fs;
use crate::keyboard::{self, Key};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Предел на размер файла: редактор держит содержимое в куче целиком.
const MAX_FILE: usize = 256 * 1024;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(name) = args.first() else {
        console.print_colored("Usage: edit <filename>\n", COLOR_RED);
        console.print_colored(
            "  full-screen editor: arrows, Ctrl+O save, Ctrl+X exit\n",
            COLOR_DARK_GRAY,
        );
        return CmdResult::Ok;
    };

    let exists = fs::read_file(name).is_ok();
    if !exists {
        if let Err(e) = fs::create_file(name) {
            console.print_colored("Error creating file: ", COLOR_RED);
            console.print(&format!("{}", e));
            console.put_char('\n');
            return CmdResult::Ok;
        }
    }

    let mut text = if exists { fs::read_file(name).unwrap_or_default() } else { Vec::new() };
    // Файлы в этой системе пишутся с CRLF (boot.log, shell.log). В редакторе
    // '\r' — управляющий символ, он не даёт перевода строки и рисуется как
    // '·', из-за чего весь лог выглядел одной строкой. Приводим к '\n'.
    if text.contains(&b'\r') {
        let mut normalized = Vec::with_capacity(text.len());
        let mut index = 0;
        while index < text.len() {
            if text[index] == b'\r' {
                if index + 1 < text.len() && text[index + 1] == b'\n' {
                    index += 1;
                    continue;
                }
                normalized.push(b'\n');
            } else {
                normalized.push(text[index]);
            }
            index += 1;
        }
        text = normalized;
    }
    if text.len() > MAX_FILE {
        console.print_colored(
            &format!("File is larger than {} KiB, opening the first part\n", MAX_FILE / 1024),
            COLOR_YELLOW,
        );
        text.truncate(MAX_FILE);
    }

    let mut editor = Editor {
        path: String::from(*name),
        text: Vec::new(),
        cursor: 0,
        top: 0,
        left: 0,
        modified: false,
        status: String::from("Ctrl+O save  Ctrl+X exit  Ctrl+K delete line"),
    };
    editor.set_text(text);
    editor.run_loop(console);

    // Полноэкранная программа обязана вернуть экран в нормальное состояние:
    // иначе приглашение печатается в той же строке, где остался курсор, и
    // накладывается на последний текст файла.
    console.clear();

    CmdResult::Ok
}

struct Editor {
    path: String,
    /// Содержимое файла; '\n' разделяет строки.
    text: Vec<u8>,
    /// Байтовое смещение курсора.
    cursor: usize,
    /// Первая видимая строка (вертикальная прокрутка).
    top: usize,
    /// Горизонтальная прокрутка в колонках.
    left: usize,
    modified: bool,
    status: String,
}

impl Editor {
    fn set_text(&mut self, text: Vec<u8>) {
        self.text = text;
        self.cursor = 0;
    }

    // ----- работа со строками -----

    /// Смещения начал строк.
    fn line_starts(&self) -> Vec<usize> {
        let mut starts = Vec::with_capacity(32);
        starts.push(0);
        for (index, byte) in self.text.iter().enumerate() {
            if *byte == b'\n' && index + 1 <= self.text.len() {
                starts.push(index + 1);
            }
        }
        starts
    }

    /// Номер строки, в которой стоит курсор.
    fn cursor_line(&self, starts: &[usize]) -> usize {
        match starts.binary_search(&self.cursor) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }

    /// Длина строки без завершающего '\n'.
    fn line_len(&self, starts: &[usize], line: usize) -> usize {
        let start = starts[line];
        let end = if line + 1 < starts.len() { starts[line + 1] - 1 } else { self.text.len() };
        end.saturating_sub(start)
    }

    /// Колонка курсора в байтах от начала строки.
    fn cursor_col(&self, starts: &[usize], line: usize) -> usize {
        self.cursor.saturating_sub(starts[line])
    }

    /// Поставить курсор в строку `line`, колонку `col` (с ограничением).
    fn place_cursor(&mut self, starts: &[usize], line: usize, col: usize) {
        let line = line.min(starts.len().saturating_sub(1));
        let start = starts[line];
        let len = self.line_len(starts, line);
        self.cursor = start + col.min(len);
    }

    // ----- редактирование -----

    fn insert_byte(&mut self, byte: u8) {
        if self.text.len() >= MAX_FILE {
            self.status = String::from("buffer is full");
            return;
        }
        self.text.insert(self.cursor, byte);
        self.cursor += 1;
        self.modified = true;
    }

    fn insert_str(&mut self, text: &str) {
        for byte in text.bytes() {
            self.insert_byte(byte);
        }
    }

    /// Удалить символ перед курсором (Backspace).
    fn delete_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.text.remove(self.cursor);
        self.modified = true;
    }

    /// Удалить символ под курсором (Ctrl+D).
    fn delete_at(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
            self.modified = true;
        }
    }

    /// Удалить текущую строку целиком (Ctrl+K).
    fn delete_line(&mut self) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        let start = starts[line];
        let end = if line + 1 < starts.len() {
            starts[line + 1]
        } else if start > 0 {
            // Последняя строка: убираем и разделитель перед ней.
            start - 1
        } else {
            self.text.len()
        };
        if start < end {
            self.text.drain(start..end.min(self.text.len()));
            self.cursor = start.min(self.text.len());
            self.modified = true;
        }
    }

    fn move_up(&mut self) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        if line > 0 {
            let col = self.cursor_col(&starts, line);
            self.place_cursor(&starts, line - 1, col);
        }
    }

    fn move_down(&mut self) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        if line + 1 < starts.len() {
            let col = self.cursor_col(&starts, line);
            self.place_cursor(&starts, line + 1, col);
        }
    }

    fn move_home(&mut self) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        self.place_cursor(&starts, line, 0);
    }

    fn move_end(&mut self) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        let len = self.line_len(&starts, line);
        self.place_cursor(&starts, line, len);
    }

    fn save(&mut self) {
        match fs::write_file(&self.path, &self.text) {
            Ok(()) => {
                self.modified = false;
                self.status = format!("saved {} bytes", self.text.len());
            }
            Err(error) => {
                self.status = format!("save failed: {}", error);
            }
        }
    }

    // ----- отрисовка -----

    /// Высота области текста.
    ///
    /// Резервируются две строки: последняя под статус и ещё одна, потому что
    /// строка ровно во всю ширину экрана переносит курсор на следующую.
    fn text_rows(console: &Console) -> usize {
        console.fb_rows().saturating_sub(2).max(1)
    }

    /// Строка статуса — последняя на экране.
    fn status_row(console: &Console) -> usize {
        console.fb_rows().saturating_sub(1)
    }

    fn draw(&mut self, console: &mut Console) {
        let starts = self.line_starts();
        let line = self.cursor_line(&starts);
        let col = self.cursor_col(&starts, line);
        let rows = Self::text_rows(console);
        let cols = console.fb_cols();

        // Держим курсор в видимой области.
        if line < self.top {
            self.top = line;
        } else if line >= self.top + rows {
            self.top = line + 1 - rows;
        }
        if col < self.left {
            self.left = col;
        } else if col >= self.left + cols.saturating_sub(1) {
            self.left = col + 1 - cols.saturating_sub(1);
        }

        console.clear();

        let total = starts.len();
        for offset in 0..rows {
            let index = self.top + offset;
            if index >= total {
                break;
            }
            let start = starts[index];
            let len = self.line_len(&starts, index);
            let from = self.left.min(len);
            let to = (self.left + cols).min(len);
            // Каждая строка файла — отдельная строка экрана. Без явной
            // установки курсора `print` продолжал печатать в той же строке,
            // и весь файл слипался в одну простыню поверх самого себя.
            console.set_cursor_position(0, offset);
            if let Ok(text) = core::str::from_utf8(&self.text[start + from..start + to]) {
                // Заменяем управляющие символы: иначе они портят экран.
                let mut cleaned = String::with_capacity(text.len());
                for ch in text.chars() {
                    cleaned.push(if ch == '\t' { ' ' } else if ch.is_control() { '·' } else { ch });
                }
                console.print(&cleaned);
            }
        }

        // Статус: имя, позиция, признак изменений, подсказка. Отдельная
        // строка внизу: текст занимает 0..rows-1, поэтому последняя строка
        // файла не перекрывается статусом.
        let status_row = Self::status_row(console);
        console.set_cursor_position(0, status_row);
        let marker = if self.modified { "*" } else { " " };
        let mut status = format!(
            "{} {}{}/{} c{}",
            self.path,
            marker,
            line + 1,
            total,
            col + 1
        );
        if status.len() > cols {
            status.truncate(cols);
        }
        console.print_colored(&status, COLOR_CYAN);
        if status_row > 0 && self.status.len() + status.len() + 2 < cols {
            console.print_colored("  ", COLOR_DARK_GRAY);
            console.print_colored(&self.status, COLOR_DARK_GRAY);
        }

        // Курсор — в текстовой области. Раньше здесь только выставлялась
        // позиция, но никто не рисовал подчёркивание: в редакторе не было
        // видно, где находится точка вставки.
        let screen_row = line.saturating_sub(self.top);
        let screen_col = col.saturating_sub(self.left).min(cols.saturating_sub(1));
        console.set_cursor_position(screen_col, screen_row);
        // draw() начинается с clear(), поэтому старый курсор уже затёрт:
        // достаточно нарисовать новый. Блочный, а не подчёркивание: под
        // символом точка вставки была не видна.
        console.reset_cursor_blink();
        console.draw_block_cursor();
    }

    /// Основной цикл. Возвращает `true`, если файл сохранён при выходе.
    fn run_loop(&mut self, console: &mut Console) {
        // Перерисовываем только после обработанной клавиши. Раньше `draw()`
        // вызывался на каждой итерации цикла опроса — то есть тысячи раз в
        // секунду; на uncached-фреймбуфере (ноутбук) это делало редактор
        // нерабочим. Пока клавиш нет, экран не меняется.
        self.draw(console);
        loop {
            let Some(key) = crate::serial::poll_key().or_else(keyboard::poll_key) else {
                console.update_cursor_blink();
                core::hint::spin_loop();
                continue;
            };

            self.status.clear();
            let mut redraw = true;
            match key {
                Key::Char('\x18') => {
                    // Ctrl+X — выход; несохранённые изменения не теряем молча.
                    if self.modified {
                        self.status = String::from("unsaved changes: Ctrl+O save, Ctrl+X again exit");
                        self.draw(console);
                        if let Some(Key::Char('\x18')) =
                            crate::serial::poll_key().or_else(keyboard::poll_key)
                        {
                            return;
                        }
                        continue;
                    }
                    return;
                }
                Key::Char('\x1B') => return,
                Key::Char('\x0F') => self.save(),
                Key::Char('\x0B') => self.delete_line(),
                Key::Char('\x04') => self.delete_at(),
                Key::Char('\x01') => self.move_home(),
                Key::Char('\x05') => self.move_end(),
                Key::Char('\x19') => {
                    // Ctrl+Y — страница вверх.
                    let rows = Self::text_rows(console);
                    let starts = self.line_starts();
                    let line = self.cursor_line(&starts);
                    let target = line.saturating_sub(rows);
                    let col = self.cursor_col(&starts, line);
                    self.place_cursor(&starts, target, col);
                }
                Key::Char('\x16') => {
                    // Ctrl+V — страница вниз.
                    let rows = Self::text_rows(console);
                    let starts = self.line_starts();
                    let line = self.cursor_line(&starts);
                    let last = starts.len().saturating_sub(1);
                    let col = self.cursor_col(&starts, line);
                    self.place_cursor(&starts, (line + rows).min(last), col);
                }
                Key::Char('\x03') | Key::Char('\x1A') => {
                    // Ctrl+C / Ctrl+Z — отмена: выходим без сохранения.
                    crate::reset_cancel();
                    self.status = String::from("cancelled");
                    return;
                }
                Key::Char(ch) if !ch.is_control() => {
                    let mut buf = [0u8; 4];
                    self.insert_str(ch.encode_utf8(&mut buf));
                }
                Key::Enter => self.insert_byte(b'\n'),
                Key::Backspace => self.delete_before(),
                Key::Tab => self.insert_str("    "),
                Key::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                Key::Right => {
                    if self.cursor < self.text.len() {
                        self.cursor += 1;
                    }
                }
                Key::Up => self.move_up(),
                Key::Down => self.move_down(),
                _ => redraw = false,
            }

            if redraw {
                self.draw(console);
            }
        }
    }
}
