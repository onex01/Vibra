// Widget — трейт виджетов GUI и реализация окна (Window).
//
// Каждый виджет умеет рисовать себя, обрабатывать клики и клавиши.
// Window — окно в стиле macOS с трафаретными кнопками, градиентом заголовка,
// скруглёнными углами и тенью.

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

/// Радиус скругления углов окна
const WINDOW_RADIUS: usize = 6;

/// Ширина и высота кнопок «светофора»
const TRAFFIC_SIZE: usize = 12;

/// Отступ между кнопками «светофора»
const TRAFFIC_GAP: usize = 6;

/// Цвета заголовка окна (градиент слева направо)
const COLOR_TITLE_START: u32 = 0x00333355;
const COLOR_TITLE_END: u32 = 0x00222244;

/// Цвета кнопок «светофора»
const COLOR_CLOSE: u32 = 0x00FF5555;
const COLOR_MINIMIZE: u32 = 0x00FFAA33;
const COLOR_MAXIMIZE: u32 = 0x0055FF55;
const COLOR_TRAFFIC_HOVER: u32 = 0x00FFFFFF;

/// Цвет тени окна
const COLOR_SHADOW: u32 = 0x00000000;

/// Ширина тени
const SHADOW_OFFSET: usize = 2;

/// Цвет области содержимого окна
const COLOR_CONTENT_BG: u32 = 0x001a1a2e;

/// Цвет активной рамки
const COLOR_ACTIVE_BORDER: u32 = 0x005599CC;

/// Цвет неактивной рамки
const COLOR_INACTIVE_BORDER: u32 = 0x00666666;

/// Состояние кнопок «светофора»
#[derive(Debug, Clone, Copy, PartialEq)]
enum TrafficButton {
    Close,
    Minimize,
    Maximize,
    None,
}

/// Окно в стиле macOS
pub struct Window {
    pub title: Vec<u8>,
    pub surface: Surface,
    pub active: bool,
    pub dragging: bool,
    pub drag_offset: (i32, i32),
    /// Есть ли тень
    has_shadow: bool,
}

impl Window {
    /// Создать новое окно
    pub fn new(title: &str, x: i32, y: i32, w: usize, h: usize) -> Self {
        // Поверхность с учётом тени (расширяем на SHADOW_OFFSET пикселей вправо и вниз)
        let sw = w + SHADOW_OFFSET;
        let sh = h + SHADOW_OFFSET;
        let mut surface = Surface::new(x, y, sw, sh);

        // Прозрачный фон (0x00000000 = прозрачный)
        surface.clear(0x00000000);

        Window {
            title: Vec::from(title.as_bytes()),
            surface,
            active: false,
            dragging: false,
            drag_offset: (0, 0),
            has_shadow: true,
        }
    }

    /// Нарисовать тень окна (2px offset на правой и нижней стороне)
    fn draw_shadow(surface: &mut Surface, w: usize, h: usize) {
        // Правая полоса тени
        for dy in 2..(h + SHADOW_OFFSET) {
            for dx in w..(w + SHADOW_OFFSET) {
                if dx < surface.width && dy < surface.height {
                    surface.put_pixel(dx, dy, COLOR_SHADOW);
                }
            }
        }
        // Нижняя полоса тени
        for dx in 2..(w + SHADOW_OFFSET) {
            for dy in h..(h + SHADOW_OFFSET) {
                if dx < surface.width && dy < surface.height {
                    surface.put_pixel(dx, dy, COLOR_SHADOW);
                }
            }
        }
    }

    /// Перерисовать содержимое окна (тень, заголовок, кнопки «светофора», содержимое)
    pub fn render(&mut self) {
        let w = self.surface.width;
        let h = self.surface.height;

        // Прозрачный фон для всей поверхности
        self.surface.clear(0x00000000);

        // Тень (рисуется первой, под окном)
        if self.has_shadow {
            Self::draw_shadow(&mut self.surface, w - SHADOW_OFFSET, h - SHADOW_OFFSET);
        }

        // Заливаем область содержимого скруглённым прямоугольником
        self.surface.fill_rounded_rect(0, 0, w - SHADOW_OFFSET, h - SHADOW_OFFSET, WINDOW_RADIUS, COLOR_CONTENT_BG);

        // Градиент заголовка (скруглённый верх)
        let title_w = w - SHADOW_OFFSET;
        for x in 0..title_w {
            let t = if title_w > 1 {
                (x as u32 * 256) / (title_w as u32 - 1)
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
                // Рисуем только если пиксель не обрезан скруглением угла
                let in_corner = Self::corner_clipped(x, y, title_w, h - SHADOW_OFFSET, WINDOW_RADIUS);
                if !in_corner {
                    self.surface.put_pixel(x, y, color);
                }
            }
        }

        // Кнопки «светофора» (macOS стиль)
        let win_w = w - SHADOW_OFFSET;
        let btn_y = (TITLE_HEIGHT - TRAFFIC_SIZE) / 2;
        let btn_start_x = 10;
        let cx = btn_start_x;
        let mx = btn_start_x + TRAFFIC_SIZE + TRAFFIC_GAP;
        let zx = mx + TRAFFIC_SIZE + TRAFFIC_GAP;

        Self::draw_traffic_button(&mut self.surface, cx, btn_y, COLOR_CLOSE);
        Self::draw_traffic_button(&mut self.surface, mx, btn_y, COLOR_MINIMIZE);
        Self::draw_traffic_button(&mut self.surface, zx, btn_y, COLOR_MAXIMIZE);

        // Разделительная линия под заголовком
        let border_color = if self.active {
            COLOR_ACTIVE_BORDER
        } else {
            COLOR_INACTIVE_BORDER
        };
        for x in 0..win_w {
            let in_corner = Self::corner_clipped(x, TITLE_HEIGHT, win_w, h - SHADOW_OFFSET, WINDOW_RADIUS);
            if !in_corner {
                self.surface.put_pixel(x, TITLE_HEIGHT, border_color);
            }
        }

        // Текст заголовка
        let title_str = core::str::from_utf8(&self.title).unwrap_or("?");
        self.surface
            .draw_text(8, (TITLE_HEIGHT - 16) / 2, title_str, framebuffer::COLOR_WHITE);

        // Рамка скруглённого прямоугольника (1px)
        Self::draw_rounded_border(&mut self.surface, win_w, h - SHADOW_OFFSET, WINDOW_RADIUS, border_color);

        self.surface.dirty = false;
    }

    /// Рисует круглую кнопку «светофора» 8x8
    fn draw_traffic_button(surface: &mut Surface, x: usize, y: usize, color: u32) {
        let radius = (TRAFFIC_SIZE / 2) as i32;
        let center = TRAFFIC_SIZE / 2;
        for dy in 0..TRAFFIC_SIZE {
            for dx in 0..TRAFFIC_SIZE {
                let diff_x = dx as i32 - center as i32;
                let diff_y = dy as i32 - center as i32;
                if diff_x * diff_x + diff_y * diff_y <= (radius - 1) * (radius - 1) {
                    surface.put_pixel(x + dx, y + dy, color);
                }
            }
        }
    }

    /// Рисует рамку скруглённого прямоугольника (1px)
    fn draw_rounded_border(surface: &mut Surface, w: usize, h: usize, radius: usize, color: u32) {
        let r = radius as i32;
        for dy in 0..h {
            for dx in 0..w {
                // Определяем, является ли пиксель граничным (расстояние до границы < 1.5px)
                let is_border = dx == 0 || dy == 0 || dx == w - 1 || dy == h - 1;
                if !is_border {
                    continue;
                }
                let in_corner = Self::corner_clipped(dx, dy, w, h, radius);
                if !in_corner {
                    // Рисуем только граничные пиксели, которые лежат на дуге или прямых
                    let is_actual_border = Self::is_on_border(dx, dy, w, h, r);
                    if is_actual_border {
                        surface.put_pixel(dx, dy, color);
                    }
                }
            }
        }
    }

    /// Проверяет, обрезается ли пиксель угловым скруглением
    #[inline]
    fn corner_clipped(dx: usize, dy: usize, w: usize, h: usize, radius: usize) -> bool {
        if radius == 0 {
            return false;
        }
        let r = radius as i32;
        let x = dx as i32;
        let y = dy as i32;
        let w_i = w as i32;
        let h_i = h as i32;

        if x < r && y < r {
            let cx = r;
            let cy = r;
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return d2 > r * r;
        }
        if x >= w_i - r && y < r {
            let cx = w_i - r - 1;
            let cy = r;
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return d2 > r * r;
        }
        if x < r && y >= h_i - r {
            let cx = r;
            let cy = h_i - r - 1;
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return d2 > r * r;
        }
        if x >= w_i - r && y >= h_i - r {
            let cx = w_i - r - 1;
            let cy = h_i - r - 1;
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            return d2 > r * r;
        }
        false
    }

    /// Проверяет, находится ли пиксель на границе (1px рамка).
    /// Сравниваем квадрат расстояния с (r±1)^2 чтобы не использовать sqrt в no_std.
    #[inline]
    fn is_on_border(dx: usize, dy: usize, w: usize, h: usize, r: i32) -> bool {
        if r <= 0 {
            return false;
        }
        // Прямые части рамки (не в угловых зонах)
        if dy == 0 || dy == h - 1 {
            let x = dx as i32;
            if x < r || x >= w as i32 - r {
                // В угловой зоне — проверяем расстояние до окружности
                let w_i = w as i32;
                let h_i = h as i32;
                let (cx, cy) = if dy == 0 {
                    if x < r { (r, r) } else { (w_i - r - 1, r) }
                } else {
                    if x < r { (r, h_i - r - 1) } else { (w_i - r - 1, h_i - r - 1) }
                };
                let d2 = (x - cx) * (x - cx) + (dy as i32 - cy) * (dy as i32 - cy);
                // Попадает на окружность если (r-1)^2 <= d2 <= (r+1)^2
                let r_min = (r - 1) * (r - 1);
                let r_max = (r + 1) * (r + 1);
                return d2 >= r_min && d2 <= r_max;
            }
            return true;
        }
        if dx == 0 || dx == w - 1 {
            let y = dy as i32;
            if y < r || y >= h as i32 - r {
                let w_i = w as i32;
                let h_i = h as i32;
                let (cx, cy) = if dx == 0 {
                    if y < r { (r, r) } else { (r, h_i - r - 1) }
                } else {
                    if y < r { (w_i - r - 1, r) } else { (w_i - r - 1, h_i - r - 1) }
                };
                let d2 = (dx as i32 - cx) * (dx as i32 - cx) + (y - cy) * (y - cy);
                let r_min = (r - 1) * (r - 1);
                let r_max = (r + 1) * (r + 1);
                return d2 >= r_min && d2 <= r_max;
            }
            return true;
        }
        false
    }

    /// Определяет, какая кнопка «светофора» нажата по локальным координатам
    fn traffic_button_at(&self, lx: usize, ly: usize) -> TrafficButton {
        let btn_y = (TITLE_HEIGHT - TRAFFIC_SIZE) / 2;
        let btn_start_x = 10;

        if ly < btn_y || ly >= btn_y + TRAFFIC_SIZE {
            return TrafficButton::None;
        }

        let cx = btn_start_x;
        let mx = btn_start_x + TRAFFIC_SIZE + TRAFFIC_GAP;
        let zx = mx + TRAFFIC_SIZE + TRAFFIC_GAP;

        if lx >= cx && lx < cx + TRAFFIC_SIZE {
            return TrafficButton::Close;
        }
        if lx >= mx && lx < mx + TRAFFIC_SIZE {
            return TrafficButton::Minimize;
        }
        if lx >= zx && lx < zx + TRAFFIC_SIZE {
            return TrafficButton::Maximize;
        }
        TrafficButton::None
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
                // Пропускаем полностью прозрачные пиксели (alpha = 0)
                if (pixel >> 24) & 0xFF != 0 || (pixel & 0x00FFFFFF) != 0 {
                    screen.put_pixel(target_x, target_y, pixel);
                } else if dx >= bw - SHADOW_OFFSET || dy >= bh - SHADOW_OFFSET {
                    // Тень — рисуем полупрозрачный (просто чёрный)
                    screen.put_pixel(target_x, target_y, pixel);
                }
            }
        }
    }

    /// Обрабатывает клик мышью
    fn handle_click(&mut self, x: usize, y: usize) {
        let (bx, by, _bw, _bh) = self.bounds();

        // Локальные координаты клика внутри поверхности
        let lx = x as i32 - bx;
        let ly = y as i32 - by;

        if lx < 0 || ly < 0 {
            return;
        }
        let lx = lx as usize;
        let ly = ly as usize;

        // Проверка кнопок «светофора»
        let btn = self.traffic_button_at(lx, ly);
        match btn {
            TrafficButton::Close => {
                // Прячем окно
                self.surface.set_position(-1000, -1000);
                self.active = false;
                return;
            }
            TrafficButton::Minimize => {
                // Минимизация — прячем окно (позже можно добавить анимацию)
                self.surface.set_position(-1000, -1000);
                self.active = false;
                return;
            }
            TrafficButton::Maximize => {
                // Максимизация — пока просто переключаем размер (toggle)
                // Реализуем простое переключение между нормальным и увеличенным размером
                // (логика будет в desktop.rs через toggle_maximize)
                return;
            }
            TrafficButton::None => {}
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
