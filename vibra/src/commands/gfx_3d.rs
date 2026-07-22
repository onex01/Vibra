/// 3D каркасный куб — вращающийся куб с проекцией и линиями Брезенхема.
/// Ctrl+Z или ESC для выхода.
use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;
use vibra_kernel::graphics::{Canvas, FpsCounter, sin_lut, cos_lut};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    vibra_kernel::reset_cancel();

    let w = console.fb_width();
    let h = console.fb_height();

    // Вершины куба (масштаб 120 единиц от центра)
    const SCALE: i32 = 120;
    let vertices: [(i32, i32, i32); 8] = [
        (-SCALE, -SCALE, -SCALE),
        (SCALE, -SCALE, -SCALE),
        (SCALE, SCALE, -SCALE),
        (-SCALE, SCALE, -SCALE),
        (-SCALE, -SCALE, SCALE),
        (SCALE, -SCALE, SCALE),
        (SCALE, SCALE, SCALE),
        (-SCALE, SCALE, SCALE),
    ];

    // Рёбра куба (12 штук)
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0), // передняя грань
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4), // задняя грань
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7), // соединительные рёбра
    ];

    const FOCAL: i32 = 350;
    const Z_OFFSET: i32 = 500;

    let center_x = (w / 2) as i32;
    let center_y = (h / 2) as i32;

    let mut angle_y: u8 = 0;
    let mut angle_x: u8 = 0;
    let mut fps = FpsCounter::new();

    console.print_colored(
        "3D Wireframe Cube — Ctrl+Z or ESC to exit\n",
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
        console.fill_rect(0, 0, w, h, 0x000a0a2a);

        // Увеличение углов поворота
        angle_y = angle_y.wrapping_add(2);
        angle_x = angle_x.wrapping_add(1);

        let sy = sin_lut(angle_y);
        let cy = cos_lut(angle_y);
        let sx = sin_lut(angle_x);
        let cx = cos_lut(angle_x);

        // Трансформация и проекция вершин
        let mut projected = [(0i32, 0i32, false); 8]; // (x, y, видима)

        for i in 0..8 {
            let (vx, vy, vz) = vertices[i];

            // Поворот вокруг оси Y
            let x1 = (vx * cy + vz * sy) / 127;
            let z1 = (-vx * sy + vz * cy) / 127;
            let y1 = vy;

            // Поворот вокруг оси X
            let y2 = (y1 * cx - z1 * sx) / 127;
            let z2 = (y1 * sx + z1 * cx) / 127;
            let x2 = x1;

            // Перспективная проекция
            let z_adj = z2 + Z_OFFSET;
            if z_adj > 10 {
                let sx_proj = center_x + x2 * FOCAL / z_adj;
                let sy_proj = center_y - y2 * FOCAL / z_adj;
                projected[i] = (sx_proj, sy_proj, true);
            }
        }

        // Рисуем рёбра через Canvas (линии Брезенхема)
        {
            let canvas = Canvas::new(&*console);
            for &(a, b) in &EDGES {
                if projected[a].2 && projected[b].2 {
                    canvas.line(
                        projected[a].0,
                        projected[a].1,
                        projected[b].0,
                        projected[b].1,
                        0x0000FF88,
                    );
                }
            }
        }

        // Рисуем вершины (точки 4x4)
        for i in 0..8 {
            if projected[i].2 {
                let (px, py, _) = projected[i];
                console.fill_rect(
                    (px as usize).saturating_sub(2),
                    (py as usize).saturating_sub(2),
                    4,
                    4,
                    0x00FFFFFF,
                );
            }
        }

        // Счётчик FPS
        fps.tick();
        fps.draw(&*console);

        vibra_kernel::task::yield_now();
    }
}
