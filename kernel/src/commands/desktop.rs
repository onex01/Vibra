// Desktop — графический режим рабочего стола.
//
// Запускает композитор с демонстрационными окнами, обрабатывает
// ввод мыши и клавиатуры, поддерживает перетаскивание окон.
// ESC возвращает в текстовый режим.

use super::CmdResult;
use crate::framebuffer::Console;
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

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    crate::println!("[GUI] Запуск графического режима...");

    // Прячем текстовый курсор
    let max_row = console.rows();
    console.set_cursor(0, max_row + 1);

    // Инициализируем курсор мыши
    cursor::init();

    // Создаём композитор
    let mut compositor = Compositor::new();

    // Окно «Системная информация»
    {
        let mut info_win = Window::new("System Info", 80, 60, 300, 200);

        // Получаем информацию о CPU
        let cpu_info = crate::cpu_info::detect();
        let brand = crate::cpu_info::brand_str(&cpu_info);
        let freq = crate::cpu_info::freq_str(&cpu_info);
        let (heap_used, heap_total) = crate::memory::heap::stats();

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
        let mouse = crate::devices::ps2_mouse::get_state();
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
        if let Some(key) = crate::keyboard::poll_key() {
            match key {
                crate::keyboard::Key::Char('\x1B') => {
                    // ESC — выход из графического режима
                    crate::println!("[GUI] Выход из графического режима");
                    // Восстанавливаем текстовый курсор
                    console.set_cursor(0, 0);
                    console.clear();
                    return CmdResult::Continue;
                }
                crate::keyboard::Key::Char(ch) => {
                    compositor.handle_key(ch as u8);
                }
                _ => {}
            }
        }

        // Рендеринг
        compositor.render(console);

        // Уступаем процессор другим задачам
        crate::task::yield_now();
    }
}
