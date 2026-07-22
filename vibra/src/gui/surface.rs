// Surface — абстракция поверхности рисования.
//
// Каждая поверхность имеет собственный буфер пикселей (Vec<u32>)
// и координаты позиции на экране. Поддерживает рисование примитивов,
// скруглённых прямоугольников и копирование в фреймбуфер через Console.

use alloc::vec::Vec;
use vibra_kernel::framebuffer::{Console, FONT_DATA, FONT_WIDTH, FONT_HEIGHT};

/// Поверхность рисования
pub struct Surface {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub z_order: i32,
    buffer: Vec<u32>,
    pub dirty: bool,
}

impl Surface {
    /// Создать новую поверхность, заполненную тёмно-серым цветом
    pub fn new(x: i32, y: i32, w: usize, h: usize) -> Self {
        let mut buffer = Vec::with_capacity(w * h);
        for _ in 0..w * h {
            buffer.push(0x00333333);
        }
        Surface {
            x,
            y,
            width: w,
            height: h,
            z_order: 0,
            buffer,
            dirty: true,
        }
    }

    /// Установить пиксель в локальных координатах
    pub fn put_pixel(&mut self, lx: usize, ly: usize, color: u32) {
        if lx < self.width && ly < self.height {
            self.buffer[ly * self.width + lx] = color;
            self.dirty = true;
        }
    }

    /// Прочитать пиксель в локальных координатах
    pub fn get_pixel(&self, lx: usize, ly: usize) -> u32 {
        if lx < self.width && ly < self.height {
            self.buffer[ly * self.width + lx]
        } else {
            0
        }
    }

    /// Заливает прямоугольник заданным цветом
    pub fn fill_rect(&mut self, lx: usize, ly: usize, w: usize, h: usize, color: u32) {
        for dy in 0..h {
            let y = ly + dy;
            if y >= self.height {
                break;
            }
            for dx in 0..w {
                let x = lx + dx;
                if x >= self.width {
                    break;
                }
                self.buffer[y * self.width + x] = color;
            }
        }
        self.dirty = true;
    }

    /// Рисует скруглённый прямоугольник (заливка).
    /// Углы скругляются с помощью проверки расстояния до центра закругляющего круга.
    pub fn fill_rounded_rect(&mut self, lx: i32, ly: i32, w: usize, h: usize, radius: usize, color: u32) {
        let r = radius as i32;
        for dy in 0..h {
            let py = ly + dy as i32;
            if py < 0 || py >= self.height as i32 {
                continue;
            }
            for dx in 0..w {
                let px = lx + dx as i32;
                if px < 0 || px >= self.width as i32 {
                    continue;
                }
                // Проверяем, попадает ли пиксель в угловую область
                let outside = Self::is_outside_rounded_rect(dx, dy, w, h, r);
                if !outside {
                    self.buffer[py as usize * self.width + px as usize] = color;
                }
            }
        }
        self.dirty = true;
    }

    /// Рисует рамку скруглённого прямоугольника (1px) с учётом прозрачности углов.
    /// Прозрачные пиксели (с alpha == 0) пропускаются.
    pub fn stroke_rounded_rect(&mut self, lx: i32, ly: i32, w: usize, h: usize, radius: usize, color: u32) {
        let r = radius as i32;
        for dy in 0..h {
            let py = ly + dy as i32;
            if py < 0 || py >= self.height as i32 {
                continue;
            }
            for dx in 0..w {
                let px = lx + dx as i32;
                if px < 0 || px >= self.width as i32 {
                    continue;
                }
                // Проверяем, что пиксель на границе (1px)
                let is_border = dx == 0 || dy == 0 || dx == w - 1 || dy == h - 1;
                if !is_border {
                    continue;
                }
                let outside = Self::is_outside_rounded_rect(dx, dy, w, h, r);
                if !outside {
                    self.buffer[py as usize * self.width + px as usize] = color;
                }
            }
        }
        self.dirty = true;
    }

    /// Определяет, находится ли пиксель (dx, dy) за пределами скруглённого прямоугольника.
    /// Пиксель считается за пределами, если он в угловой области и лежит вне окружности.
    #[inline]
    fn is_outside_rounded_rect(dx: usize, dy: usize, w: usize, h: usize, r: i32) -> bool {
        if r <= 0 {
            return false;
        }
        let x = dx as i32;
        let y = dy as i32;
        let w_i = w as i32;
        let h_i = h as i32;

        // Верхний левый угол
        if x < r && y < r {
            let cx = r;
            let cy = r;
            let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return dist_sq > r * r;
        }
        // Верхний правый угол
        if x >= w_i - r && y < r {
            let cx = w_i - r - 1;
            let cy = r;
            let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return dist_sq > r * r;
        }
        // Нижний левый угол
        if x < r && y >= h_i - r {
            let cx = r;
            let cy = h_i - r - 1;
            let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return dist_sq > r * r;
        }
        // Нижний правый угол
        if x >= w_i - r && y >= h_i - r {
            let cx = w_i - r - 1;
            let cy = h_i - r - 1;
            let dist_sq = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return dist_sq > r * r;
        }
        false
    }

    /// Копирует содержимое поверхности в фреймбуфер
    pub fn blit_to(&self, target: &Console) {
        if self.x < 0 || self.y < 0 {
            return;
        }
        target.blit(
            self.x as usize,
            self.y as usize,
            &self.buffer,
            self.width,
            self.height,
        );
    }

    /// Переместить поверхность
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.dirty = true;
    }

    /// Проверяет, содержит ли поверхность данную точку (в экранных координатах)
    pub fn contains(&self, px: usize, py: usize) -> bool {
        if self.x < 0 || self.y < 0 {
            return false;
        }
        let sx = self.x as usize;
        let sy = self.y as usize;
        px >= sx && px < sx + self.width && py >= sy && py < sy + self.height
    }

    /// Рисует строку текста bitmap-шрифтом ядра
    pub fn draw_text(&mut self, lx: usize, ly: usize, text: &str, fg: u32) {
        for (i, ch) in text.chars().enumerate() {
            let char_x = lx + i * FONT_WIDTH;
            if char_x + FONT_WIDTH > self.width {
                break;
            }
            if ly + FONT_HEIGHT > self.height {
                break;
            }
            if ch >= ' ' && ch <= '~' {
                let font_index = (ch as usize) - 32;
                let glyph = &FONT_DATA[font_index];
                for dy in 0..FONT_HEIGHT {
                    let y = ly + dy;
                    if y >= self.height {
                        break;
                    }
                    let line = glyph[dy];
                    for dx in 0..FONT_WIDTH {
                        if (line >> (7 - dx)) & 1 == 1 {
                            self.buffer[y * self.width + char_x + dx] = fg;
                        }
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// Очищает поверхность заданным цветом
    pub fn clear(&mut self, color: u32) {
        for pixel in self.buffer.iter_mut() {
            *pixel = color;
        }
        self.dirty = true;
    }

    /// Возвращает ссылку на внутренний буфер пикселей
    pub fn buffer(&self) -> &[u32] {
        &self.buffer
    }
}
