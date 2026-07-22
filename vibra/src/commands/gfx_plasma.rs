/// Эффект плазмы — классический демошенный эффект с синусоидальными волнами.
/// Виртуальное разрешение 320×240, back buffer, предвычисленная палитра.
/// Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::SINE_LUT;
use vibra_kernel::graphics::FpsCounter;

/// Предвычисленная палитра плазмы: 256 цветов
fn build_plasma_palette() -> [u32; 256] {
    let mut palette = [0u32; 256];
    for i in 0..256 {
        let r = SINE_LUT[i] as u32;
        let g = SINE_LUT[(i + 85) & 255] as u32;
        let b = SINE_LUT[(i + 170) & 255] as u32;
        palette[i] = (r << 16) | (g << 8) | b;
    }
    palette
}

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    console.enable_back_buffer();
    console.set_virtual_resolution(320, 240);

    let w = console.fb_width();   // 320
    let h = console.fb_height();  // 240

    // Предвычисление палитры (256 цветов)
    let palette = build_plasma_palette();

    let mut tick: u32 = 0;
    let mut fps = FpsCounter::new();

    console.draw_text_at(
        0,
        4,
        "Plasma Effect - Ctrl+Z or ESC",
        vibra_kernel::framebuffer::COLOR_CYAN,
        0x00000000,
    );
    console.flip();

    loop {
        // Проверка отмены (Ctrl+Z)
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

        // Проверка ESC
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

        let t = tick;

        // Вычисление плазмы для каждого пикселя (только 320×240 = 76800 пикселей!)
        for y in 0..h {
            for x in 0..w {
                let xt = (x as u32).wrapping_add(t);
                let yt = (y as u32).wrapping_add(t);
                let xy2 = (((x + y) / 2) as u32).wrapping_add(t.wrapping_mul(3) / 2);
                let xmy = (x as i32 / 2).wrapping_sub(y as i32 / 2);
                let xmy2 = (xmy as u32).wrapping_add(t.wrapping_mul(2));

                // Четыре синусоидальных волны с разными фазами
                let v1 = SINE_LUT[xt as usize & 255] as i32;
                let v2 = SINE_LUT[yt as usize & 255] as i32;
                let v3 = SINE_LUT[xy2 as usize & 255] as i32;
                let v4 = SINE_LUT[xmy2 as usize & 255] as i32;

                // Усреднение и нормализация к 0-255
                let v = ((v1 + v2 + v3 + v4) / 4 + 128).max(0).min(255) as usize;
                let color = palette[v];
                console.put_pixel(x, y, color);
            }
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        // Копируем back buffer → framebuffer
        console.flip();

        tick = tick.wrapping_add(1);
        vibra_kernel::task::yield_now();
    }
}
