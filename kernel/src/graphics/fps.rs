/// Счётчик кадров в секунду (FPS)
use crate::framebuffer::Console;

pub struct FpsCounter {
    frame_count: u64,
    last_ms: u64,
    current_fps: u32,
}

impl FpsCounter {
    pub fn new() -> Self {
        FpsCounter {
            frame_count: 0,
            last_ms: crate::timer::uptime_ms(),
            current_fps: 0,
        }
    }

    /// Вызывать один раз за кадр. Обновляет FPS раз в секунду по монотонным
    /// часам (IRQ-тики на реальном железе могут не доставляться).
    pub fn tick(&mut self) {
        self.frame_count += 1;
        let now = crate::timer::uptime_ms();
        let elapsed = now.wrapping_sub(self.last_ms);
        if elapsed >= 1000 {
            self.current_fps = (self.frame_count * 1000 / elapsed) as u32;
            self.frame_count = 0;
            self.last_ms = now;
        }
    }

    /// Текущее значение FPS
    pub fn fps(&self) -> u32 {
        self.current_fps
    }

    /// Нарисовать счётчик FPS в правом верхнем углу экрана
    pub fn draw(&self, console: &Console) {
        let w = console.fb_width();
        let fps_text = alloc::format!("{} FPS", self.current_fps);
        let text_width = fps_text.len() * 8; // FONT_WIDTH = 8
        let bg_color: u32 = 0x00111111;

        let x = if w > text_width + 8 {
            w - text_width - 8
        } else {
            0
        };
        let y = 4;

        console.fill_rect(x.saturating_sub(4), y, text_width + 8, 16, bg_color);
        console.draw_text_at(x, y, &fps_text, 0x00FFFFFF, bg_color);
    }
}
