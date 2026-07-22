// Surface — абстракция поверхности рисования.
//
// Каждая поверхность имеет собственный буфер пикселей (Vec<u32>)
// и координаты позиции на экране. Поддерживает рисование примитивов
// и копирование в фреймбуфер через Console.

use alloc::vec::Vec;
use crate::framebuffer::{Console, FONT_DATA, FONT_WIDTH, FONT_HEIGHT};

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
}
