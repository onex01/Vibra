/// Простая графическая демка — рисует движущиеся фигуры на framebuffer.
/// Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    let w = console.fb_width();
    let h = console.fb_height();

    // Фон
    console.fill_rect(0, 0, w, h, 0x000a0a2a);

    let mut frame: u32 = 0;
    let mut box_x: i32 = 0;
    let mut box_y: i32 = 0;
    let mut dx: i32 = 3;
    let mut dy: i32 = 2;
    let box_size = 40;

    console.print_colored("GFX Demo — Ctrl+Z or ESC to exit\n", vibra_kernel::framebuffer::COLOR_CYAN);

    loop {
        // Проверяем отмену
        if vibra_kernel::is_cancelled() {
            vibra_kernel::reset_cancel();
            console.print_colored("\n[GFX] Demo cancelled\n", vibra_kernel::framebuffer::COLOR_YELLOW);
            return CmdResult::Ok;
        }

        // Проверяем ESC через keyboard
        if let Some(key) = vibra_kernel::keyboard::poll_key() {
            match key {
                vibra_kernel::keyboard::Key::Char('\x1B') => {
                    console.print_colored("\n[GFX] Demo exited\n", vibra_kernel::framebuffer::COLOR_GREEN);
                    return CmdResult::Ok;
                }
                vibra_kernel::keyboard::Key::Char('\x1A') => {
                    vibra_kernel::request_cancel();
                    console.print_colored("\n[GFX] Demo cancelled\n", vibra_kernel::framebuffer::COLOR_YELLOW);
                    return CmdResult::Ok;
                }
                _ => {}
            }
        }

        // Сохраняем пиксели под коробкой (простой подход — рисуем на чистом фоне)
        // Рисуем чёрный след
        let old_x = box_x as usize;
        let old_y = box_y as usize;
        if old_x < w && old_y < h {
            let clear_size = box_size + 4;
            if old_x + clear_size <= w && old_y + clear_size <= h {
                console.fill_rect(old_x, old_y, clear_size, clear_size, 0x000a0a2a);
            }
        }

        // Двигаем коробку
        box_x += dx;
        box_y += dy;

        // Отскок от стенок
        if box_x + box_size as i32 >= w as i32 || box_x <= 0 {
            dx = -dx;
            box_x += dx;
        }
        if box_y + box_size as i32 >= h as i32 || box_y <= 0 {
            dy = -dy;
            box_y += dy;
        }

        // Рисуем коробку с цветом по кадру
        let color = match frame % 6 {
            0 => 0x00FF3333, // красный
            1 => 0x0033FF33, // зелёный
            2 => 0x003333FF, // синий
            3 => 0x00FFFF33, // жёлтый
            4 => 0x00FF33FF, // пурпурный
            _ => 0x0033FFFF, // циан
        };

        let ux = box_x as usize;
        let uy = box_y as usize;
        if ux + box_size <= w && uy + box_size <= h {
            console.fill_rect(ux, uy, box_size, box_size, color);
            // Рамка
            console.draw_rect(ux, uy, box_size, box_size, 0x00FFFFFF);
        }

        // FPS counter в углу
        let fps_text = alloc::format!("Frame: {}", frame);
        console.fill_rect(w - 120, 4, 120, 16, 0x000a0a2a);
        console.draw_text_at(w - 120, 4, &fps_text, 0x00FFFFFF, 0x000a0a2a);

        frame += 1;

        // Yield — не блокируем scheduler
        vibra_kernel::task::yield_now();
    }
}
