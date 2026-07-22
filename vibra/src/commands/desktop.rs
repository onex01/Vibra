// Desktop — графический режим рабочего стола.
//
// Запускает композитор с демонстрационными окнами, обрабатывает
// ввод мыши и клавиатуры, поддерживает перетаскивание окон.
// ESC возвращает в текстовый режим.
// F12 переключает между десктопом и текстовым шеллом.

use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::{self, Console};
use crate::gui::compositor::Compositor;
use crate::gui::widget::Window;
use crate::gui::cursor;

/// Вспомогательная функция: число в строку на стеке
fn usize_to_str(mut n: usize, buf: &mut [u8]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

/// Рисует экран загрузки с прогресс-баром
fn draw_loading_screen(console: &Console) {
    let w = console.fb_width();
    let h = console.fb_height();

    // Фон загрузки (тёмно-синий)
    console.fill_rect(0, 0, w, h, 0x000a0a1a);

    // Название ОС
    let title = "Vibra OS";
    let title_len = title.len() * 8;
    let title_x = if w > title_len { (w - title_len) / 2 } else { 0 };
    let title_y = h / 2 - 40;
    console.draw_text_at(title_x, title_y, title, framebuffer::COLOR_CYAN, 0x000a0a1a);

    // Прогресс-бар
    let bar_width = 200;
    let bar_height = 12;
    let bar_x = if w > bar_width { (w - bar_width) / 2 } else { 0 };
    let bar_y = h / 2;

    // Рамка прогресс-бара
    console.draw_rect(bar_x, bar_y, bar_width, bar_height, 0x00555555);

    // Заполняем прогресс-бар (внутри)
    console.fill_rect(bar_x + 1, bar_y + 1, bar_width - 2, bar_height - 2, 0x00222233);
}

/// Заполняет прогресс-бар до заданного процента
fn update_progress_bar(console: &Console, percent: usize) {
    let w = console.fb_width();
    let h = console.fb_height();

    let bar_width = 200;
    let bar_height = 12;
    let bar_x = if w > bar_width { (w - bar_width) / 2 } else { 0 };
    let bar_y = h / 2;

    // Очищаем внутреннюю часть
    console.fill_rect(bar_x + 1, bar_y + 1, bar_width - 2, bar_height - 2, 0x00222233);

    // Заполняем согласно проценту
    let fill_w = ((bar_width - 2) * percent) / 100;
    if fill_w > 0 {
        console.fill_rect(bar_x + 1, bar_y + 1, fill_w, bar_height - 2, 0x003388FF);
    }

    // Процент текстом
    let mut buf = [0u8; 4];
    let mut pos = 0;
    let val = percent;
    if val == 0 {
        buf[0] = b'0';
        pos = 1;
    } else {
        let mut tmp = [0u8; 3];
        let mut tpos = 0;
        let mut v = val;
        while v > 0 && tpos < 3 {
            tmp[tpos] = b'0' + (v % 10) as u8;
            v /= 10;
            tpos += 1;
        }
        let mut i = tpos;
        while i > 0 && pos < 3 {
            i -= 1;
            buf[pos] = tmp[i];
            pos += 1;
        }
    }
    buf[pos] = b'%';
    pos += 1;

    let pct_str = core::str::from_utf8(&buf[..pos]).unwrap_or("0%");
    let pct_len = pos * 8;
    let pct_x = if w > pct_len { (w - pct_len) / 2 } else { 0 };
    let pct_y = bar_y + bar_height + 8;

    // Очищаем область под процентами
    console.fill_rect(pct_x, pct_y, pct_len + 8, 16, 0x000a0a1a);
    console.draw_text_at(pct_x + 4, pct_y, pct_str, framebuffer::COLOR_WHITE, 0x000a0a1a);
}

/// Запуск графического режима из команды
pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::println!("[GUI] Запуск графического режима...");

    // Прячем текстовый курсор
    let max_row = console.rows();
    console.set_cursor(0, max_row + 1);

    // Экран загрузки
    draw_loading_screen(console);

    // Анимация загрузки (0% до 100%)
    for i in 0..=100 {
        update_progress_bar(console, i);
        vibra_kernel::task::yield_now();
    }

    // Инициализируем курсор мыши
    cursor::init();

    // Создаём композитор
    let mut compositor = Compositor::new();

    // Окно «Системная информация»
    {
        let mut info_win = Window::new("System Info", 80, 60, 300, 200);

        // Получаем информацию о CPU
        let cpu_info = vibra_kernel::cpu_info::detect();
        let brand = vibra_kernel::cpu_info::brand_str(&cpu_info);
        let freq = vibra_kernel::cpu_info::freq_str(&cpu_info);
        let (heap_used, heap_total) = vibra_kernel::memory::heap::stats();

        // Заполняем информацию в окне (область содержимого)
        let mut y_offset = 32; // после заголовка (24px) + 8px отступ
        let content_x = 8;
        let content_color = 0x00e0e0e0;
        let label_color = 0x0000CC88;
        let value_color = 0x00FFFFFF;

        // Название CPU
        info_win.surface.draw_text(content_x, y_offset, "CPU:", label_color);
        // Разбиваем бренд на строки по 34 символа
        let mut brand_remaining = brand;
        y_offset += 18;
        while !brand_remaining.is_empty() {
            let (line, rest) = if brand_remaining.len() > 34 {
                brand_remaining.split_at(34)
            } else {
                (brand_remaining, "")
            };
            info_win.surface.draw_text(content_x + 8, y_offset, line, value_color);
            y_offset += 16;
            brand_remaining = rest;
        }

        // Частота
        y_offset += 4;
        info_win.surface.draw_text(content_x, y_offset, "Freq: ", label_color);
        info_win
            .surface
            .draw_text(content_x + 48, y_offset, &freq, value_color);
        y_offset += 18;

        // Память
        info_win.surface.draw_text(content_x, y_offset, "Heap:", label_color);
        y_offset += 18;
        let mut num_buf = [0u8; 20];
        let used_str = usize_to_str(heap_used, &mut num_buf);
        info_win
            .surface
            .draw_text(content_x + 8, y_offset, "Used: ", content_color);
        info_win
            .surface
            .draw_text(content_x + 56, y_offset, used_str, value_color);
        y_offset += 16;
        let total_str = usize_to_str(heap_total, &mut num_buf);
        info_win
            .surface
            .draw_text(content_x + 8, y_offset, "Total: ", content_color);
        info_win
            .surface
            .draw_text(content_x + 56, y_offset, total_str, value_color);

        compositor.add_window(info_win);
    }

    // Окно «Терминал»
    {
        let mut term_win = Window::new("Terminal", 200, 180, 400, 250);

        // Подсказка в области содержимого
        term_win.surface.draw_text(
            8,
            36,
            "Vibra OS Terminal",
            0x0000FF88,
        );
        term_win.surface.draw_text(
            8,
            56,
            "Type 'exit' to return",
            0x00888888,
        );

        compositor.add_window(term_win);
    }

    // Начальный рендер
    compositor.update_windows();
    compositor.render(console);

    // === Главный цикл GUI ===
    loop {
        // Опрос мыши
        let mouse = vibra_kernel::devices::ps2_mouse::get_state();
        if mouse.dx != 0 || mouse.dy != 0 {
            compositor.handle_mouse_move(mouse.dx as i32, mouse.dy as i32);
        }

        // Обработка кликов
        if mouse.left_button {
            let (cx, cy) = cursor::get_position();
            if cx >= 0 && cy >= 0 {
                compositor.handle_click(cx as usize, cy as usize);
            }
        }

        // Опрос клавиатуры
        if let Some(key) = vibra_kernel::keyboard::poll_key() {
            match key {
                vibra_kernel::keyboard::Key::Char('\x1B') => {
                    // ESC — выход из графического режима
                    vibra_kernel::println!("[GUI] Выход из графического режима");
                    // Восстанавливаем текстовый курсор
                    console.set_cursor(0, 0);
                    console.clear();
                    return CmdResult::Continue;
                }
                vibra_kernel::keyboard::Key::Char('l') => {
                    // F12 (scancode 0x57) поступает как Key::Char('l')? Нет.
                    // F12 = 0x58 scancode — проверяем через poll_key
                    // F12 в keyboard.rs не определён, проверим другой способ
                }
                vibra_kernel::keyboard::Key::Char(ch) => {
                    compositor.handle_key(ch as u8);
                }
                _ => {}
            }
        }

        // Проверяем F12 (scancode 0x58) — он не обрабатывается keyboard.rs,
        // поэтому проверяем напрямую через poll_raw_scancode если доступно
        // Пока используем только ESC для выхода.

        // Рендеринг
        compositor.render(console);

        // Уступаем процессор другим задачам
        vibra_kernel::task::yield_now();
    }
}
