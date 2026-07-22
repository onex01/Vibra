/// Демо система частиц — 200 частиц с гравитацией и отскоком от стенок.
/// Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::FpsCounter;
use vibra_kernel::graphics::color::hsv_to_rgb;
use alloc::vec::Vec;

struct Particle {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    color: u32,
}

struct Rng {
    state: u32,
}

impl Rng {
    fn new(seed: u32) -> Self {
        Rng { state: seed }
    }

    fn next(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state
    }
}

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    let w = console.fb_width();
    let h = console.fb_height();

    console.fill_rect(0, 0, w, h, 0x00000000);

    // Инициализация генератора случайных чисел на основе системного таймера
    let ticks = vibra_kernel::interrupts::idt::TICKS.load(core::sync::atomic::Ordering::Relaxed);
    let mut rng = Rng::new(ticks as u32);

    // Создание 200 частиц со случайными параметрами
    let mut particles: Vec<Particle> = Vec::with_capacity(200);
    for _ in 0..200 {
        let x = (rng.next() % w as u32) as i32;
        let y = (rng.next() % h as u32) as i32;
        let vx = ((rng.next() % 60) as i32) - 30;
        let vy = ((rng.next() % 60) as i32) - 30;
        let hue = (rng.next() % 360) as f32;
        let color = hsv_to_rgb(hue, 1.0, 1.0);
        particles.push(Particle {
            x,
            y,
            vx,
            vy,
            color,
        });
    }

    let mut fps = FpsCounter::new();

    console.print_colored(
        "Particle System — Ctrl+Z or ESC to exit\n",
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

        // Очистка экрана
        console.fill_rect(0, 0, w, h, 0x00000000);

        // Обновление и отрисовка частиц
        for p in particles.iter_mut() {
            // Гравитация: +1 к vy (0.1 пикселей/тик² * 10 для точности)
            p.vy += 1;

            // Обновление позиции
            p.x += p.vx / 10;
            p.y += p.vy / 10;

            // Отскок от стенок
            if p.x < 0 {
                p.x = 0;
                p.vx = p.vx.abs();
            }
            if p.x >= w as i32 - 3 {
                p.x = w as i32 - 3;
                p.vx = -p.vx.abs();
            }
            if p.y < 0 {
                p.y = 0;
                p.vy = p.vy.abs();
            }
            if p.y >= h as i32 - 3 {
                p.y = h as i32 - 3;
                p.vy = -p.vy.abs();
            }

            // Рисуем частицу как 3x3 квадрат
            console.fill_rect(p.x as usize, p.y as usize, 3, 3, p.color);
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        vibra_kernel::task::yield_now();
    }
}
