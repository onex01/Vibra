/// Простая графическая демка — рисует движущиеся фигуры на framebuffer.
/// Использует back buffer и виртуальное разрешение 320×240. Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::FpsCounter;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    console.enable_back_buffer();
    console.set_virtual_resolution(320, 240);

    let w = console.fb_width();
    let h = console.fb_height();

    // Фон
    console.fill_rect(0, 0, w, h, 0x000a0a2a);
    // Надпись в виртуальных координатах (через draw_text_at → put_pixel → back buffer)
    console.draw_text_at(
        0,
        4,
        "GFX Demo - Ctrl+Z or ESC",
        vibra_kernel::framebuffer::COLOR_CYAN,
        0x000a0a2a,
    );
    console.flip();

    let mut box_x: i32 = 0;
    let mut box_y: i32 = 0;
    let mut dx: i32 = 3;
    let mut dy: i32 = 2;
    let box_size = 40;
    let mut frame: u32 = 0;
    let mut fps = FpsCounter::new();

    loop {
        // Проверяем отмену (Ctrl+Z)
        if vibra_kernel::is_cancelled() {
            vibra_kernel::reset_cancel();
            console.disable_back_buffer();
            console.restore_text_mode();
            console.print_colored(
                "[GFX] Demo отменён\n",
                vibra_kernel::framebuffer::COLOR_YELLOW,
            );
            return CmdResult::Ok;
        }

        // Проверяем ESC через keyboard
        if let Some(key) = vibra_kernel::keyboard::poll_key() {
            match key {
                vibra_kernel::keyboard::Key::Char('\x1B') => {
                    console.disable_back_buffer();
                    console.restore_text_mode();
                    console.print_colored(
                        "[GFX] Demo завершён\n",
                        vibra_kernel::framebuffer::COLOR_GREEN,
                    );
                    return CmdResult::Ok;
                }
                vibra_kernel::keyboard::Key::Char('\x1A') => {
                    vibra_kernel::request_cancel();
                    console.disable_back_buffer();
                    console.restore_text_mode();
                    console.print_colored(
                        "[GFX] Demo отменён\n",
                        vibra_kernel::framebuffer::COLOR_YELLOW,
                    );
                    return CmdResult::Ok;
                }
                _ => {}
            }
        }

        // Стираем след коробки
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
            0 => 0x00FF3333,
            1 => 0x0033FF33,
            2 => 0x003333FF,
            3 => 0x00FFFF33,
            4 => 0x00FF33FF,
            _ => 0x0033FFFF,
        };

        let ux = box_x as usize;
        let uy = box_y as usize;
        if ux + box_size <= w && uy + box_size <= h {
            console.fill_rect(ux, uy, box_size, box_size, color);
            console.draw_rect(ux, uy, box_size, box_size, 0x00FFFFFF);
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        frame += 1;

        // Копируем back buffer → framebuffer
        console.flip();

        // Yield — не блокируем scheduler
        vibra_kernel::task::yield_now();
    }
}
