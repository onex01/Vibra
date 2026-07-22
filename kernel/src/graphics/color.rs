/// Цветовые утилиты для графического API

/// Цветовые константы (формат 0x00RRGGBB)
pub const RED: u32 = 0x00FF0000;
pub const GREEN: u32 = 0x0000FF00;
pub const BLUE: u32 = 0x000000FF;
pub const WHITE: u32 = 0x00FFFFFF;
pub const BLACK: u32 = 0x00000000;
pub const CYAN: u32 = 0x0000FFFF;
pub const YELLOW: u32 = 0x00FFFF00;
pub const MAGENTA: u32 = 0x00FF00FF;

/// Создать цвет из компонент RGB (формат 0x00RRGGBB)
pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Линейная интерполяция между двумя цветами (t: 0.0..1.0).
/// Использует целочисленную арифметику: t*256 как целое.
pub fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let t_fixed = (t * 256.0) as i32;
    let t_fixed = if t_fixed < 0 { 0 } else if t_fixed > 256 { 256 } else { t_fixed };

    let ar = ((a >> 16) & 0xFF) as i32;
    let ag = ((a >> 8) & 0xFF) as i32;
    let ab = (a & 0xFF) as i32;

    let br = ((b >> 16) & 0xFF) as i32;
    let bg = ((b >> 8) & 0xFF) as i32;
    let bb = (b & 0xFF) as i32;

    let rr = ar + (br - ar) * t_fixed / 256;
    let rg = ag + (bg - ag) * t_fixed / 256;
    let rb = ab + (bb - ab) * t_fixed / 256;

    ((rr as u32) << 16) | ((rg as u32) << 8) | (rb as u32)
}

/// Преобразование HSV в RGB (h: 0..360, s: 0..1, v: 0..1).
/// Все внутренние вычисления на целых числах, без libm.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    // Конвертируем в целые числа сразу
    let h_i = ((h * 1536.0 / 360.0) as i32).max(0).min(1535) as u32; // 0-1535
    let s_i = (s * 255.0) as u32; // 0-255
    let v_i = (v * 255.0) as u32; // 0-255

    if s_i == 0 {
        return rgb(v_i as u8, v_i as u8, v_i as u8);
    }

    let sector = (h_i >> 8) & 7; // 0-5
    let f = h_i & 0xFF; // 0-255

    let p = v_i * (255 - s_i) / 255;
    let q = v_i * (255 - s_i * f / 255) / 255;
    let t = v_i * (255 - s_i * (255 - f) / 255) / 255;

    let (r, g, b) = match sector {
        0 => (v_i, t, p),
        1 => (q, v_i, p),
        2 => (p, v_i, t),
        3 => (p, q, v_i),
        4 => (t, p, v_i),
        _ => (v_i, p, q),
    };

    rgb(r as u8, g as u8, b as u8)
}
