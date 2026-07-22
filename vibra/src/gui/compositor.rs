// Compositor — композитор окон.
//
// Управляет списком окон и поверхностей, обрабатывает ввод мыши/клавиатуры,
// выполняет z-сортировку и рендеринг на фреймбуфер через Console.
// Рисует верхнюю панель (top bar), док (dock) и счётчик FPS.

use alloc::vec::Vec;
use vibra_kernel::framebuffer::{self, Console};
use super::surface::Surface;
use super::widget::{Widget, Window};
use super::cursor;

/// Высота верхней панели (px)
const TOP_BAR_HEIGHT: usize = 24;

/// Высота дока (px)
const DOCK_HEIGHT: usize = 52;

/// Размер иконок в доке (px)
const DOCK_ICON_SIZE: usize = 12;

/// Отступ между иконками в доке (px)
const DOCK_GAP: usize = 4;

/// Цвет фона верхней панели
const COLOR_TOP_BAR: u32 = 0x001a1a2e;

/// Цвет фона дока (полупрозрачный)
const COLOR_DOCK_BG: u32 = 0xBB222233;

/// Цвета иконок дока
const COLOR_FINDER_ICON: u32 = 0x003388FF;
const COLOR_TERMINAL_ICON: u32 = 0x00333333;
const COLOR_SETTINGS_ICON: u32 = 0x00888888;
const COLOR_INFO_ICON: u32 = 0x0033AA33;

/// Цвет рамки иконки терминала
const COLOR_TERMINAL_BORDER: u32 = 0x00666666;

/// Цвет белой точки (активное приложение)
const COLOR_ACTIVE_DOT: u32 = 0x00FFFFFF;

/// Количество иконок в доке
const DOCK_ICON_COUNT: usize = 4;

/// Композитор
pub struct Compositor {
    pub surfaces: Vec<Surface>,
    pub windows: Vec<Window>,
    pub active_window: Option<usize>,
    /// Счётчик кадров для FPS
    frame_count: u64,
    /// Последнее значение TICKS при обновлении FPS
    last_fps_tick: u64,
    /// Текущее значение FPS
    current_fps: u64,
    /// Активные приложения в доке (по индексу иконки)
    dock_active: [bool; DOCK_ICON_COUNT],
}

impl Compositor {
    /// Создать пустой композитор
    pub fn new() -> Self {
        Compositor {
            surfaces: Vec::new(),
            windows: Vec::new(),
            active_window: None,
            frame_count: 0,
            last_fps_tick: 0,
            current_fps: 0,
            dock_active: [false; DOCK_ICON_COUNT],
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

    /// Рисует градиент фона рабочего стола (тёмно-синий)
    fn draw_background(console: &Console) {
        let w = console.fb_width();
        let h = console.fb_height();
        for y in 0..h {
            let t = if h > 1 {
                (y as u32 * 256) / (h as u32 - 1)
            } else {
                0
            };
            let inv_t = 256 - t;
            // Градиент от 0x0a0a1a (верх) до 0x1a1a3e (низ)
            let r = ((0x0a * inv_t + 0x1a * t) / 256) as u32;
            let g = ((0x0a * inv_t + 0x1a * t) / 256) as u32;
            let b = ((0x1a * inv_t + 0x3e * t) / 256) as u32;
            let color = (r << 16) | (g << 8) | b;
            console.fill_rect(0, y, w, 1, color);
        }
    }

    /// Рисует верхнюю панель
    fn draw_top_bar(console: &Console) {
        let w = console.fb_width();
        let h = console.fb_height();

        // Фон панели
        let bar_h = TOP_BAR_HEIGHT.min(h);
        console.fill_rect(0, 0, w, bar_h, COLOR_TOP_BAR);

        // Иконка приложения (синий квадрат 8x8) — слева
        let icon_x = 8;
        let icon_y = (bar_h - 8) / 2;
        console.fill_rect(icon_x, icon_y, 8, 8, 0x003388FF);

        // Время — центр
        let time_str = "12:00";
        let time_len = time_str.len() * 8; // 8px на символ
        let time_x = if w > time_len {
            (w - time_len) / 2
        } else {
            0
        };
        // Фон под временем
        console.fill_rect(time_x, 0, time_len + 8, bar_h, COLOR_TOP_BAR);
        console.draw_text_at(time_x + 4, (bar_h - 16) / 2, time_str, framebuffer::COLOR_WHITE, COLOR_TOP_BAR);

        // Кнопка питания (красный квадрат 8x8) — справа
        let power_x = w.saturating_sub(16);
        let power_y = (bar_h - 8) / 2;
        console.fill_rect(power_x, power_y, 8, 8, 0x00FF4444);
    }

    /// Рисует док внизу экрана
    fn draw_dock(console: &Console, dock_active: &[bool; DOCK_ICON_COUNT]) {
        let w = console.fb_width();
        let h = console.fb_height();

        // Позиция дока
        let dock_total_w = DOCK_ICON_COUNT * DOCK_ICON_SIZE + (DOCK_ICON_COUNT - 1) * DOCK_GAP + 16;
        let dock_x = if w > dock_total_w {
            (w - dock_total_w) / 2
        } else {
            0
        };
        let dock_y = h.saturating_sub(DOCK_HEIGHT);

        // Фон дока (полупрозрачный)
        console.fill_rect(dock_x, dock_y, dock_total_w, DOCK_HEIGHT, COLOR_DOCK_BG);

        // Иконки
        let icon_colors = [
            COLOR_FINDER_ICON,
            COLOR_TERMINAL_ICON,
            COLOR_SETTINGS_ICON,
            COLOR_INFO_ICON,
        ];

        let icon_y_center = dock_y + (DOCK_HEIGHT - DOCK_ICON_SIZE) / 2;

        for i in 0..DOCK_ICON_COUNT {
            let ix = dock_x + 8 + i * (DOCK_ICON_SIZE + DOCK_GAP);
            let iy = icon_y_center;
            let color = icon_colors[i];

            // Рисуем иконку
            console.fill_rect(ix, iy, DOCK_ICON_SIZE, DOCK_ICON_SIZE, color);

            // Рамка для терминала (индекс 1)
            if i == 1 {
                console.draw_rect(ix, iy, DOCK_ICON_SIZE, DOCK_ICON_SIZE, COLOR_TERMINAL_BORDER);
            }

            // Белая точка для активных приложений
            if dock_active[i] {
                let dot_x = ix + (DOCK_ICON_SIZE - 4) / 2;
                let dot_y = iy + DOCK_ICON_SIZE + 2;
                console.fill_rect(dot_x, dot_y, 4, 4, COLOR_ACTIVE_DOT);
            }
        }
    }

    /// Рендеринг всего на фреймбуфер
    pub fn render(&mut self, console: &Console) {
        // Обновляем FPS
        self.update_fps();

        // Стираем старый курсор
        cursor::undraw(console);

        // Градиент фона рабочего стола
        Self::draw_background(console);

        // Рисуем standalone-поверхности
        for surface in &self.surfaces {
            surface.blit_to(console);
        }

        // Рисуем окна (поверх фона, под UI-элементами)
        for window in &self.windows {
            window.surface.blit_to(console);
        }

        // Верхняя панель
        Self::draw_top_bar(console);

        // Док
        Self::draw_dock(console, &self.dock_active);

        // Счётчик FPS (верхний правый угол)
        self.draw_fps(console);

        // Рисуем курсор поверх всего
        cursor::draw(console);
    }

    /// Обновляет счётчик FPS
    fn update_fps(&mut self) {
        let ticks = vibra_kernel::interrupts::idt::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        self.frame_count += 1;

        // TICKS обновляется каждые ~50ms (freq = 20 Hz у PIT таймера, или иначе)
        // Обновляем FPS каждые ~100 тиков
        if ticks >= self.last_fps_tick + 100 {
            let elapsed = ticks - self.last_fps_tick;
            if elapsed > 0 {
                self.current_fps = (self.frame_count * 1000) / (elapsed * 50); // 50ms per tick
            }
            self.frame_count = 0;
            self.last_fps_tick = ticks;
        }
    }

    /// Рисует FPS в правом верхнем углу
    fn draw_fps(&self, console: &Console) {
        let w = console.fb_width();
        let fps = self.current_fps;

        // Формируем строку "XX FPS"
        let mut buf = [0u8; 8];
        let mut pos = 0;

        // Число FPS
        if fps == 0 {
            buf[0] = b'0';
            pos = 1;
        } else {
            let mut tmp = [0u8; 4];
            let mut tpos = 0;
            let mut val = fps;
            while val > 0 && tpos < 4 {
                tmp[tpos] = b'0' + (val % 10) as u8;
                val /= 10;
                tpos += 1;
            }
            // Разворачиваем
            let mut i = tpos;
            while i > 0 && pos < 4 {
                i -= 1;
                buf[pos] = tmp[i];
                pos += 1;
            }
        }

        // Пробел
        if pos < 8 {
            buf[pos] = b' ';
            pos += 1;
        }

        // "FPS"
        if pos + 3 <= 8 {
            buf[pos] = b'F';
            buf[pos + 1] = b'P';
            buf[pos + 2] = b'S';
            pos += 3;
        }

        let fps_str = core::str::from_utf8(&buf[..pos]).unwrap_or("0 FPS");
        let fps_len = pos * 8; // 8px на символ
        let x = w.saturating_sub(fps_len + 8);
        let y = TOP_BAR_HEIGHT + 4;

        // Фон под FPS
        console.fill_rect(x, y, fps_len + 8, 16, 0x001a1a2e);
        console.draw_text_at(x + 4, y, fps_str, framebuffer::COLOR_WHITE, 0x001a1a2e);
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
        // Клик на верхней панели — игнорируем (пока)
        if y < TOP_BAR_HEIGHT {
            return;
        }

        // Клик на доке — игнорируем (пока)
        if y >= 480_usize.saturating_sub(DOCK_HEIGHT) {
            return;
        }

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
