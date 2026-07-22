pub mod gop;
pub mod pcivga;

use alloc::vec::Vec;
use spin::Mutex;

/// Унифицированный интерфейс для всех графических бэкендов
pub trait DisplayBackend {
    fn name(&self) -> &str;
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn pitch(&self) -> u32;  // байт на строку
    fn bpp(&self) -> u8;     // бит на пиксель
    fn fb_ptr(&self) -> *mut u8;
    fn fb_size(&self) -> usize;
    fn pixel_format(&self) -> PixelFormat;
    fn set_mode(&mut self, width: u32, height: u32, bpp: u8) -> bool;
    fn flush(&mut self) {}   // для double-buffering (no-op если нет)
}

/// Формат пикселей
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb888,   // 0x00RRGGBB
    Bgr888,   // 0x00BBGGRR (Limine по умолчанию)
    Rgb565,   // 16bpp
    Indexed8, // 8bpp палитра
    Text,     // VGA текстовый режим
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            PixelFormat::Rgb888 | PixelFormat::Bgr888 => 4,
            PixelFormat::Rgb565 => 2,
            PixelFormat::Indexed8 => 1,
            PixelFormat::Text => 0, // не pixel-based
        }
    }

    /// Имя формата для вывода
    pub fn name(&self) -> &'static str {
        match self {
            PixelFormat::Rgb888 => "RGB888",
            PixelFormat::Bgr888 => "BGR888",
            PixelFormat::Rgb565 => "RGB565",
            PixelFormat::Indexed8 => "Indexed8",
            PixelFormat::Text => "Text",
        }
    }
}

/// Конвертация цвета из Rgb888 в native format
pub fn color_to_native(color: u32, format: PixelFormat) -> u32 {
    match format {
        PixelFormat::Rgb888 => color,
        PixelFormat::Bgr888 => {
            let r = (color >> 16) & 0xFF;
            let g = (color >> 8) & 0xFF;
            let b = color & 0xFF;
            (b << 16) | (g << 8) | r
        }
        PixelFormat::Rgb565 => {
            let r = ((color >> 16) & 0xFF) as u16;
            let g = ((color >> 8) & 0xFF) as u16;
            let b = (color & 0xFF) as u16;
            ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u32
        }
        _ => color,
    }
}

/// Конвертация native pixel в Rgb888
pub fn native_to_color(native: u32, format: PixelFormat) -> u32 {
    match format {
        PixelFormat::Rgb888 => native,
        PixelFormat::Bgr888 => {
            let b = (native >> 16) & 0xFF;
            let g = (native >> 8) & 0xFF;
            let r = native & 0xFF;
            (r << 16) | (g << 8) | b
        }
        PixelFormat::Rgb565 => {
            let rgb565 = native as u16;
            let r = ((rgb565 >> 11) & 0x1F) as u32;
            let g = ((rgb565 >> 5) & 0x3F) as u32;
            let b = (rgb565 & 0x1F) as u32;
            (r << 3) << 16 | (g << 2) << 8 | (b << 3)
        }
        _ => native,
    }
}

/// Display Manager — singleton для управления графическими бэкендами
pub struct DisplayManager {
    backends: Vec<DisplayBackendEntry>,
    active: usize,
}

struct DisplayBackendEntry {
    name: alloc::string::String,
    width: u32,
    height: u32,
    bpp: u8,
    format: PixelFormat,
}

static DISPLAY: Mutex<Option<DisplayManager>> = Mutex::new(None);

/// Инициализация display manager
pub fn init() {
    let mgr = DisplayManager {
        backends: Vec::new(),
        active: 0,
    };
    *DISPLAY.lock() = Some(mgr);
    crate::println!("[DISPLAY] Дисплей менеджер инициализирован");
}

/// Зарегистрировать графический бэкенд
pub fn register_backend(name: &str, width: u32, height: u32, bpp: u8, format: PixelFormat) {
    if let Some(ref mut mgr) = *DISPLAY.lock() {
        let entry = DisplayBackendEntry {
            name: alloc::string::String::from(name),
            width,
            height,
            bpp,
            format,
        };
        mgr.backends.push(entry);
        crate::println!(
            "[DISPLAY] Зарегистрирован бэкенд: {} {}x{} {}bpp {:?}",
            name, width, height, bpp, format
        );
    }
}

/// Получить количество зарегистрированных бэкендов
pub fn backend_count() -> usize {
    if let Some(ref mgr) = *DISPLAY.lock() {
        mgr.backends.len()
    } else {
        0
    }
}

/// Получить информацию о бэкенде по индексу
pub fn get_backend_info(index: usize) -> Option<(alloc::string::String, u32, u32, u8, PixelFormat)> {
    if let Some(ref mgr) = *DISPLAY.lock() {
        if let Some(entry) = mgr.backends.get(index) {
            return Some((entry.name.clone(), entry.width, entry.height, entry.bpp, entry.format));
        }
    }
    None
}

/// Получить текущий активный формат
pub fn get_active_format() -> PixelFormat {
    if let Some(ref mgr) = *DISPLAY.lock() {
        if let Some(entry) = mgr.backends.get(mgr.active) {
            return entry.format;
        }
    }
    // По умолчанию: BGR888 (Limine)
    PixelFormat::Bgr888
}
