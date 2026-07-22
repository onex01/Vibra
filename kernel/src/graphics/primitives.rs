/// Графические примитивы поверх framebuffer.
/// Все алгоритмы используют только целочисленную арифметику.
use crate::framebuffer::Console;
use super::color;

pub struct Canvas<'a> {
    console: &'a Console,
}

impl<'a> Canvas<'a> {
    pub fn new(console: &'a Console) -> Self {
        Canvas { console }
    }

    /// Алгоритм Брезенхема для рисования отрезка (целочисленный)
    pub fn line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut x = x0;
        let mut y = y0;
        let w = self.console.fb_width() as i32;
        let h = self.console.fb_height() as i32;

        loop {
            if x >= 0 && x < w && y >= 0 && y < h {
                self.console.put_pixel(x as usize, y as usize, color);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Алгоритм средней точки для контура окружности
    pub fn circle(&self, cx: i32, cy: i32, r: i32, color: u32) {
        if r <= 0 {
            return;
        }
        let mut x = r;
        let mut y = 0;
        let mut d = 1 - r;
        let w = self.console.fb_width() as i32;
        let h = self.console.fb_height() as i32;

        while x >= y {
            plot_circle_points(self.console, cx, cy, x, y, w, h, color);
            y += 1;
            if d <= 0 {
                d += 2 * y + 1;
            } else {
                x -= 1;
                d += 2 * (y - x) + 1;
            }
        }
    }

    /// Заливка окружности
    pub fn fill_circle(&self, cx: i32, cy: i32, r: i32, color: u32) {
        if r <= 0 {
            return;
        }
        let r_sq = r * r;
        let w = self.console.fb_width() as i32;
        let h = self.console.fb_height() as i32;

        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r_sq {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < w && py >= 0 && py < h {
                        self.console.put_pixel(px as usize, py as usize, color);
                    }
                }
            }
        }
    }

    /// Заливка треугольника (алгоритм барицентрических координат)
    pub fn triangle(
        &self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
    ) {
        let w = self.console.fb_width() as i32;
        let h = self.console.fb_height() as i32;

        let min_x = x0.min(x1).min(x2).max(0);
        let max_x = x0.max(x1).max(x2).min(w - 1);
        let min_y = y0.min(y1).min(y2).max(0);
        let max_y = y0.max(y1).max(y2).min(h - 1);

        for py in min_y..=max_y {
            for px in min_x..=max_x {
                if point_in_triangle(px, py, x0, y0, x1, y1, x2, y2) {
                    self.console.put_pixel(px as usize, py as usize, color);
                }
            }
        }
    }

    /// Вертикальный градиент (сверху color_top, снизу color_bot)
    pub fn gradient_fill(
        &self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color_top: u32,
        color_bot: u32,
    ) {
        if h <= 0 {
            return;
        }
        for dy in 0..h {
            let t = dy as f32 / h as f32;
            let c = color::lerp(color_top, color_bot, t);
            let py = y + dy;
            if py >= 0 {
                self.console
                    .fill_rect(x as usize, py as usize, w as usize, 1, c);
            }
        }
    }

    /// Толстый контур окружности
    pub fn draw_circle_outline(
        &self,
        cx: i32,
        cy: i32,
        r: i32,
        color: u32,
        thickness: i32,
    ) {
        let half = thickness / 2;
        for t in -half..=half {
            let ri = r + t;
            if ri > 0 {
                self.circle(cx, cy, ri, color);
            }
        }
    }
}

/// Вспомогательная функция: рисует 8 симметричных точек окружности
fn plot_circle_points(
    console: &Console,
    cx: i32,
    cy: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
) {
    let points = [
        (cx + x, cy + y),
        (cx - x, cy + y),
        (cx + x, cy - y),
        (cx - x, cy - y),
        (cx + y, cy + x),
        (cx - y, cy + x),
        (cx + y, cy - x),
        (cx - y, cy - x),
    ];

    for (px, py) in points {
        if px >= 0 && px < w && py >= 0 && py < h {
            console.put_pixel(px as usize, py as usize, color);
        }
    }
}

/// Знак функции для барицентрических координат
fn sign_val(px: i32, py: i32, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
    (px - x1) * (y0 - y1) - (x0 - x1) * (py - y1)
}

/// Проверка: находится ли точка внутри треугольника
fn point_in_triangle(
    px: i32,
    py: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> bool {
    let d1 = sign_val(px, py, x0, y0, x1, y1);
    let d2 = sign_val(px, py, x1, y1, x2, y2);
    let d3 = sign_val(px, py, x2, y2, x0, y0);
    let has_neg = (d1 < 0) || (d2 < 0) || (d3 < 0);
    let has_pos = (d1 > 0) || (d2 > 0) || (d3 > 0);
    !(has_neg && has_pos)
}
