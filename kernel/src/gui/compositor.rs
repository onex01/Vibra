// Compositor — композитор окон.
//
// Управляет списком окон и поверхностей, обрабатывает ввод мыши/клавиатуры,
// выполняет z-сортировку и рендеринг на фреймбуфер через Console.

use alloc::vec::Vec;
use crate::framebuffer::Console;
use super::surface::Surface;
use super::widget::{Widget, Window};
use super::cursor;

/// Композитор
pub struct Compositor {
    pub surfaces: Vec<Surface>,
    pub windows: Vec<Window>,
    pub active_window: Option<usize>,
}

impl Compositor {
    /// Создать пустой композитор
    pub fn new() -> Self {
        Compositor {
            surfaces: Vec::new(),
            windows: Vec::new(),
            active_window: None,
        }
    }

    /// Добавить окно в композитор
    pub fn add_window(&mut self, window: Window) {
        self.windows.push(window);
        let idx = self.windows.len() - 1;
        self.windows[idx].render();
        // Если первое окно — делаем активным
        if self.active_window.is_none() {
            self.windows[idx].active = true;
            self.active_window = Some(idx);
        }
    }

    /// Добавить standalone-поверхность
    pub fn add_surface(&mut self, surface: Surface) {
        self.surfaces.push(surface);
    }

    /// Перерисовать все окна (вызывать при изменении состояния)
    pub fn update_windows(&mut self) {
        for i in 0..self.windows.len() {
            self.windows[i].render();
        }
    }

    /// Рендеринг всего на фреймбуфер
    pub fn render(&self, console: &Console) {
        // Стираем старый курсор
        cursor::undraw(console);

        // Заливаем фон рабочего стола
        console.fill_rect(0, 0, console.fb_width(), console.fb_height(), 0x001a1a2e);

        // Рисуем standalone-поверхности
        for surface in &self.surfaces {
            surface.blit_to(console);
        }

        // Рисуем окна
        for window in &self.windows {
            window.surface.blit_to(console);
        }

        // Рисуем курсор поверх всего
        cursor::draw(console);
    }

    /// Обработка движения мыши
    pub fn handle_mouse_move(&mut self, dx: i32, dy: i32) {
        let (cx, cy) = cursor::get_position();
        let new_x = (cx + dx).max(0);
        let new_y = (cy + dy).max(0);
        cursor::move_to(new_x, new_y);

        // Обработка перетаскивания окна
        if let Some(idx) = self.active_window {
            if idx < self.windows.len() && self.windows[idx].dragging {
                let new_win_x = new_x - self.windows[idx].drag_offset.0;
                let new_win_y = new_y - self.windows[idx].drag_offset.1;
                self.windows[idx]
                    .surface
                    .set_position(new_win_x, new_win_y);
                self.windows[idx].render();
            }
        }
    }

    /// Обработка клика мыши
    pub fn handle_click(&mut self, x: usize, y: usize) {
        // Ищем верхнее окно (последнее в списке = поверх остальных)
        let mut clicked_idx: Option<usize> = None;
        for i in (0..self.windows.len()).rev() {
            if self.windows[i].surface.contains(x, y) {
                clicked_idx = Some(i);
                break;
            }
        }

        // Сбрасываем перетаскивание у предыдущего активного
        if let Some(old_idx) = self.active_window {
            if old_idx < self.windows.len() {
                self.windows[old_idx].dragging = false;
                self.windows[old_idx].active = false;
            }
        }

        if let Some(idx) = clicked_idx {
            self.windows[idx].handle_click(x, y);
            self.windows[idx].active = true;
            self.active_window = Some(idx);
            self.windows[idx].render();
        } else {
            self.active_window = None;
        }

        // Перерисовываем рамки всех окон
        for i in 0..self.windows.len() {
            self.windows[i].render();
        }
    }

    /// Обработка нажатия клавиши — пересылка активному окну
    pub fn handle_key(&mut self, key: u8) {
        if let Some(idx) = self.active_window {
            if idx < self.windows.len() {
                self.windows[idx].handle_key(key);
            }
        }
    }
}
