#![no_std]
#![no_main]

extern crate alloc;

use vibra_kernel as kernel;

mod gui;
mod commands;

/// Команды vibra ОС (GUI + расширенные)
fn register_os_commands() {
    kernel::commands::register_command(kernel::commands::Command {
        name: "cpuid",
        help: "show CPU information",
        func: commands::cpuid::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "memmap",
        help: "show memory map",
        func: commands::memmap::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "diskinfo",
        help: "show AHCI disk info",
        func: commands::diskinfo::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "lsusb",
        help: "list USB controllers",
        func: commands::lsusb::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "desktop",
        help: "launch graphical desktop",
        func: commands::desktop::run,
    });
}

/// Рисует экран загрузки с прогресс-баром
fn draw_loading_screen(console: &kernel::framebuffer::Console) {
    let w = console.fb_width();
    let h = console.fb_height();

    // Фон загрузки (тёмно-синий)
    console.fill_rect(0, 0, w, h, 0x000a0a1a);

    // Название ОС
    let title = "Vibra OS";
    let title_len = title.len() * 8;
    let title_x = if w > title_len { (w - title_len) / 2 } else { 0 };
    let title_y = h / 2 - 40;
    console.draw_text_at(title_x, title_y, title, kernel::framebuffer::COLOR_CYAN, 0x000a0a1a);

    // Прогресс-бар
    let bar_width = 200;
    let bar_height = 12;
    let bar_x = if w > bar_width { (w - bar_width) / 2 } else { 0 };
    let bar_y = h / 2;

    // Рамка прогресс-бара
    console.draw_rect(bar_x, bar_y, bar_width, bar_height, 0x00555555);

    // Очищаем внутреннюю часть
    console.fill_rect(bar_x + 1, bar_y + 1, bar_width - 2, bar_height - 2, 0x00222233);
}

/// Обновляет прогресс-бар до заданного процента
fn update_progress_bar(console: &kernel::framebuffer::Console, percent: usize) {
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
    console.draw_text_at(pct_x + 4, pct_y, pct_str, kernel::framebuffer::COLOR_WHITE, 0x000a0a1a);
}

/// Запуск десктопа — главная точка входа GUI
fn desktop_main(bc: kernel::BootConsole) -> ! {
    let mut console: kernel::framebuffer::Console = bc.console;

    // Прячем текстовый курсор
    let max_row = console.rows();
    console.set_cursor(0, max_row + 1);

    // Экран загрузки с прогресс-баром
    draw_loading_screen(&console);
    for i in 0..=100 {
        update_progress_bar(&console, i);
        kernel::task::yield_now();
    }

    // Инициализируем курсор мыши
    gui::cursor::init();

    // Создаём композитор
    let mut compositor = gui::compositor::Compositor::new();

    // Окно «Системная информация»
    {
        let mut info_win = gui::widget::Window::new("System Info", 80, 60, 300, 200);

        let cpu_info = kernel::cpu_info::detect();
        let brand = kernel::cpu_info::brand_str(&cpu_info);
        let freq = kernel::cpu_info::freq_str(&cpu_info);
        let (heap_used, heap_total) = kernel::memory::heap::stats();

        let mut y_offset = 32;
        let content_x = 8;
        let label_color = 0x0000CC88;
        let value_color = 0x00FFFFFF;

        info_win.surface.draw_text(content_x, y_offset, "CPU:", label_color);
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

        y_offset += 4;
        info_win.surface.draw_text(content_x, y_offset, "Freq: ", label_color);
        info_win.surface.draw_text(content_x + 48, y_offset, &freq, value_color);
        y_offset += 18;

        info_win.surface.draw_text(content_x, y_offset, "Heap:", label_color);
        y_offset += 18;
        let mut num_buf = [0u8; 20];
        let used_str = usize_to_str(heap_used, &mut num_buf);
        info_win.surface.draw_text(content_x + 8, y_offset, "Used: ", 0x00e0e0e0);
        info_win.surface.draw_text(content_x + 56, y_offset, used_str, value_color);
        y_offset += 16;
        let total_str = usize_to_str(heap_total, &mut num_buf);
        info_win.surface.draw_text(content_x + 8, y_offset, "Total: ", 0x00e0e0e0);
        info_win.surface.draw_text(content_x + 56, y_offset, total_str, value_color);

        compositor.add_window(info_win);
    }

    // Окно «Терминал»
    {
        let mut term_win = gui::widget::Window::new("Terminal", 200, 180, 400, 250);
        term_win.surface.draw_text(8, 36, "Vibra OS Terminal", 0x0000FF88);
        term_win.surface.draw_text(8, 56, "ESC to return to shell", 0x00888888);
        compositor.add_window(term_win);
    }

    // Начальный рендер
    compositor.update_windows();
    compositor.render(&console);

    // === Главный цикл GUI ===
    loop {
        // Опрос мыши
        let mouse = kernel::devices::ps2_mouse::get_state();
        if mouse.dx != 0 || mouse.dy != 0 {
            compositor.handle_mouse_move(mouse.dx as i32, mouse.dy as i32);
        }

        if mouse.left_button {
            let (cx, cy) = gui::cursor::get_position();
            if cx >= 0 && cy >= 0 {
                compositor.handle_click(cx as usize, cy as usize);
            }
        }

        // Опрос клавиатуры
        if let Some(key) = kernel::keyboard::poll_key() {
            match key {
                kernel::keyboard::Key::Char('\x1B') => {
                    // ESC — переключение в текстовый шелл
                    kernel::println!("[GUI] Переключение в текстовый режим");
                    console.set_cursor(0, 0);
                    console.clear();
                    // Запускаем шелл (never returns)
                    kernel::shell_loop(kernel::BootConsole { console });
                }
                kernel::keyboard::Key::Char(ch) => {
                    compositor.handle_key(ch as u8);
                }
                _ => {}
            }
        }

        // Рендеринг
        compositor.render(&console);

        // Уступаем процессор другим задачам
        kernel::task::yield_now();
    }
}

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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Инициализация ядра (hardware, memory, drivers, scheduler)
    let bc = kernel::init();

    // Регистрируем OS-команды (GUI, desktop, extended)
    register_os_commands();

    // Запускаем десктоп (ESC переключает в шелл)
    desktop_main(bc);
}
