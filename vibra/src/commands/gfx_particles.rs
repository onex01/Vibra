/// Демо система частиц — 60 частиц с гравитацией и отскоком от стенок.
/// Виртуальное разрешение 320×240, back buffer. Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::FpsCounter;
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

// Простые цвета частиц (без HSV — быстрее)
const PARTICLE_COLORS: [u32; 8] = [
    0x00FF4444,
    0x0044FF44,
    0x004444FF,
    0x00FFFF44,
    0x00FF44FF,
    0x0044FFFF,
    0x00FF8844,
    0x0044FF88,
];

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    console.enable_back_buffer();
    console.set_virtual_resolution(320, 240);

    let w = console.fb_width();
    let h = console.fb_height();

    console.fill_rect(0, 0, w, h, 0x00000000);

    // Инициализация генератора случайных чисел на основе системного таймера
    let ticks = vibra_kernel::interrupts::idt::TICKS.load(core::sync::atomic::Ordering::Relaxed);
    let mut rng = Rng::new(ticks as u32);

    // Создание 60 частиц со случайными параметрами
    let mut particles: Vec<Particle> = Vec::with_capacity(60);
    for i in 0..60 {
        let x = (rng.next() % w as u32) as i32;
        let y = (rng.next() % h as u32) as i32;
        let vx = ((rng.next() % 40) as i32) - 20;
        let vy = ((rng.next() % 40) as i32) - 20;
        let color = PARTICLE_COLORS[i % PARTICLE_COLORS.len()];
        particles.push(Particle {
            x,
            y,
            vx,
            vy,
            color,
        });
    }

    let mut fps = FpsCounter::new();

    console.draw_text_at(
        0,
        4,
        "Particles (60) - Ctrl+Z or ESC",
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

        // Очистка экрана
        console.fill_rect(0, 0, w, h, 0x00000000);

        // Обновление и отрисовка частиц
        for p in particles.iter_mut() {
            // Гравитация: +1 к vy
            p.vy += 1;

            // Обновление позиции
            p.x += p.vx / 10;
            p.y += p.vy / 10;

            // Отскок от стенок
            if p.x < 0 {
                p.x = 0;
                p.vx = p.vx.abs();
            }
            if p.x >= w as i32 - 2 {
                p.x = w as i32 - 2;
                p.vx = -p.vx.abs();
            }
            if p.y < 0 {
                p.y = 0;
                p.vy = p.vy.abs();
            }
            if p.y >= h as i32 - 2 {
                p.y = h as i32 - 2;
                p.vy = -p.vy.abs();
            }

            // Рисуем частицу как 2x2 квадрат
            console.fill_rect(p.x as usize, p.y as usize, 2, 2, p.color);
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        // Копируем back buffer → framebuffer
        console.flip();

        vibra_kernel::task::yield_now();
    }
}
