/// Эффект плазмы — классический демошенный эффект с синусоидальными волнами.
/// Пиксельный расчёт каждого кадра. Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::SINE_LUT;
use vibra_kernel::graphics::FpsCounter;

/// Карта цвета плазмы: значение v (0-255) → цвет радуги
fn plasma_color(v: usize) -> u32 {
    let r = SINE_LUT[v & 255] as u32;
    let g = SINE_LUT[(v + 85) & 255] as u32;
    let b = SINE_LUT[(v + 170) & 255] as u32;
    (r << 16) | (g << 8) | b
}

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    let w = console.fb_width();
    let h = console.fb_height();

    let mut tick: u32 = 0;
    let mut fps = FpsCounter::new();

    console.print_colored(
        "Plasma Effect — Ctrl+Z or ESC to exit\n",
        vibra_kernel::framebuffer::COLOR_CYAN,
    );

    loop {
        // Проверка отмены (Ctrl+Z)
        if vibra_kernel::is_cancelled() {
            vibra_kernel::reset_cancel();
            console.print_colored(
                "\n[GFX] Demo cancelled\n",
                vibra_kernel::framebuffer::COLOR_YELLOW,
            );
            console.restore_text_mode();
            return CmdResult::Ok;
        }

        // Проверка ESC
        if let Some(key) = vibra_kernel::keyboard::poll_key() {
            match key {
                vibra_kernel::keyboard::Key::Char('\x1B') => {
                    console.print_colored(
                        "\n[GFX] Demo exited\n",
                        vibra_kernel::framebuffer::COLOR_GREEN,
                    );
                    console.restore_text_mode();
            return CmdResult::Ok;
                }
                vibra_kernel::keyboard::Key::Char('\x1A') => {
                    vibra_kernel::request_cancel();
                    console.print_colored(
                        "\n[GFX] Demo cancelled\n",
                        vibra_kernel::framebuffer::COLOR_YELLOW,
                    );
                    console.restore_text_mode();
            return CmdResult::Ok;
                }
                _ => {}
            }
        }

        let t = tick;

        // Вычисление плазмы для каждого пикселя
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
                let color = plasma_color(v);
                console.put_pixel(x, y, color);
            }
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        tick = tick.wrapping_add(1);
        vibra_kernel::task::yield_now();
    }
}
