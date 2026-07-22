// Widget — трейт виджетов GUI и реализация окна (Window).
//
// Каждый виджет умеет рисовать себя, обрабатывать клики и клавиши.
// Window — базовое окно с заголовком, кнопкой закрытия и областью содержимого.

use alloc::vec::Vec;
use vibra_kernel::framebuffer;
use super::surface::Surface;

/// Трейт виджета
pub trait Widget {
    fn draw(&self, surface: &mut Surface);
    fn handle_click(&mut self, x: usize, y: usize);
    fn handle_key(&mut self, key: u8);
    fn bounds(&self) -> (i32, i32, usize, usize);
}

/// Высота заголовка окна
const TITLE_HEIGHT: usize = 24;

/// Размер кнопки закрытия
const CLOSE_BTN_SIZE: usize = 20;

/// Цвета окна
const COLOR_TITLE_START: u32 = 0x003366;
const COLOR_TITLE_END: u32 = 0x005599;
const COLOR_CLOSE_BTN: u32 = 0x00CC3333;
const COLOR_CLOSE_BTN_HOVER: u32 = 0x00FF4444;
const COLOR_CONTENT_BG: u32 = 0x001a1a2e;
const COLOR_ACTIVE_BORDER: u32 = 0x005599CC;
const COLOR_INACTIVE_BORDER: u32 = 0x00666666;

/// Окно
pub struct Window {
    pub title: Vec<u8>,
    pub surface: Surface,
    pub active: bool,
    pub dragging: bool,
    pub drag_offset: (i32, i32),
}

impl Window {
    /// Создать новое окно
    pub fn new(title: &str, x: i32, y: i32, w: usize, h: usize) -> Self {
        let mut surface = Surface::new(x, y, w, h);
        // Заливаем область содержимого
        surface.fill_rect(0, TITLE_HEIGHT, w, h - TITLE_HEIGHT, COLOR_CONTENT_BG);
        // Рисуем рамку
        Self::draw_borders(&mut surface, w, h, COLOR_ACTIVE_BORDER);

        Window {
            title: Vec::from(title.as_bytes()),
            surface,
            active: false,
            dragging: false,
            drag_offset: (0, 0),
        }
    }

    /// Рисует рамки окна
    fn draw_borders(surface: &mut Surface, w: usize, h: usize, color: u32) {
        if w == 0 || h == 0 {
            return;
        }
        surface.fill_rect(0, 0, w, 1, color);
        surface.fill_rect(0, h.saturating_sub(1), w, 1, color);
        surface.fill_rect(0, 0, 1, h, color);
        surface.fill_rect(w.saturating_sub(1), 0, 1, h, color);
    }

    /// Перерисовать содержимое окна (заголовок, кнопки, рамки)
    pub fn render(&mut self) {
        let w = self.surface.width;
        let h = self.surface.height;

        // Очищаем поверхность
        self.surface.clear(COLOR_CONTENT_BG);

        // Рисуем градиент заголовка
        for x in 0..w {
            let t = if w > 1 {
                (x as u32 * 256) / (w as u32 - 1)
            } else {
                0
            };
            let inv_t = 256 - t;
            let r = (((COLOR_TITLE_START >> 16) & 0xFF) * inv_t
                + ((COLOR_TITLE_END >> 16) & 0xFF) * t)
                / 256;
            let g = (((COLOR_TITLE_START >> 8) & 0xFF) * inv_t
                + ((COLOR_TITLE_END >> 8) & 0xFF) * t)
                / 256;
            let b =
                ((COLOR_TITLE_START & 0xFF) * inv_t + (COLOR_TITLE_END & 0xFF) * t) / 256;
            let color = (r << 16) | (g << 8) | b;
            for y in 0..TITLE_HEIGHT {
                self.surface.put_pixel(x, y, color);
            }
        }

        // Кнопка закрытия
        if w > CLOSE_BTN_SIZE + 8 {
            let cb_x = w - CLOSE_BTN_SIZE - 4;
            let cb_y = (TITLE_HEIGHT - CLOSE_BTN_SIZE) / 2;
            self.surface
                .fill_rect(cb_x, cb_y, CLOSE_BTN_SIZE, CLOSE_BTN_SIZE, COLOR_CLOSE_BTN);

            // Рисуем крестик (X) белым цветом
            let cx = cb_x + 4;
            let cy = cb_y + 4;
            let cs = CLOSE_BTN_SIZE - 8;
            for i in 0..cs {
                self.surface.put_pixel(cx + i, cy + i, framebuffer::COLOR_WHITE);
                self.surface
                    .put_pixel(cx + cs - 1 - i, cy + i, framebuffer::COLOR_WHITE);
            }
        }

        // Текст заголовка
        let title_str = core::str::from_utf8(&self.title).unwrap_or("?");
        self.surface
            .draw_text(8, (TITLE_HEIGHT - 16) / 2, title_str, framebuffer::COLOR_WHITE);

        // Рамка
        let border_color = if self.active {
            COLOR_ACTIVE_BORDER
        } else {
            COLOR_INACTIVE_BORDER
        };
        Self::draw_borders(&mut self.surface, w, h, border_color);

        self.surface.dirty = false;
    }
}

impl Widget for Window {
    /// Рисует окно на целевой поверхности (экране)
    fn draw(&self, screen: &mut Surface) {
        let (bx, by, bw, bh) = self.bounds();
        let sx = if bx < 0 { 0 } else { bx as usize };
        let sy = if by < 0 { 0 } else { by as usize };

        for dy in 0..bh {
            let target_y = sy + dy;
            if target_y >= screen.height {
                break;
            }
            for dx in 0..bw {
                let target_x = sx + dx;
                if target_x >= screen.width {
                    break;
                }
                let pixel = self.surface.get_pixel(dx, dy);
                screen.put_pixel(target_x, target_y, pixel);
            }
        }
    }

    /// Обрабатывает клик мышью
    fn handle_click(&mut self, x: usize, y: usize) {
        let (bx, by, bw, _bh) = self.bounds();

        // Проверка кнопки закрытия
        if bw > CLOSE_BTN_SIZE + 8 {
            let cb_x = bx + (bw as i32) - (CLOSE_BTN_SIZE as i32) - 4;
            let cb_y = by + ((TITLE_HEIGHT as i32 - CLOSE_BTN_SIZE as i32) / 2);
            let cb_size = CLOSE_BTN_SIZE as i32;
            let xi = x as i32;
            let yi = y as i32;

            if xi >= cb_x && xi < cb_x + cb_size && yi >= cb_y && yi < cb_y + cb_size {
                // Нажата кнопка закрытия — прячем окно
                self.surface.set_position(-1000, -1000);
                self.active = false;
                return;
            }
        }

        // Перетаскивание через заголовок
        {
            let yi = y as i32;
            let xi = x as i32;
            let title_h = TITLE_HEIGHT as i32;
            if yi >= by && yi < by + title_h {
                self.dragging = true;
                self.drag_offset = (xi - bx, yi - by);
                self.active = true;
            }
        }
    }

    /// Обрабатывает нажатие клавиши
    fn handle_key(&mut self, _key: u8) {
        // Пока не обрабатываем клавиши в окне
    }

    /// Возвращает (x, y, width, height) окна
    fn bounds(&self) -> (i32, i32, usize, usize) {
        (
            self.surface.x,
            self.surface.y,
            self.surface.width,
            self.surface.height,
        )
    }
}
