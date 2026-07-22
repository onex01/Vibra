/// UEFI GOP backend — основной для UEFI boot
use super::{DisplayBackend, PixelFormat};

pub struct GopBackend {
    fb_addr: *mut u8,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u8,
    format: PixelFormat,
}

unsafe impl Send for GopBackend {}
unsafe impl Sync for GopBackend {}

impl GopBackend {
    pub fn new(
        fb_addr: *mut u8,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u8,
        format: PixelFormat,
    ) -> Self {
        Self {
            fb_addr,
            width,
            height,
            pitch,
            bpp,
            format,
        }
    }
}

/// Зарегистрировать GOP бэкенд из Limine framebuffer
pub fn register_from_framebuffer(fb: &limine::framebuffer::Framebuffer) {
    let addr = fb.address() as *mut u8;
    let w = fb.width as u32;
    let h = fb.height as u32;
    let pitch = fb.pitch as u32;
    let bpp = fb.bpp as u8;

    // Limine по умолчанию использует BGR888
    let format = PixelFormat::Bgr888;

    super::register_backend("GOP (UEFI)", w, h, bpp, format);

    // Создаём экземпляр (регистрация уже выполнена через register_backend)
    let _gop = GopBackend::new(addr, w, h, pitch, bpp, format);
}

impl DisplayBackend for GopBackend {
    fn name(&self) -> &str {
        "GOP (UEFI)"
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn pitch(&self) -> u32 {
        self.pitch
    }
    fn bpp(&self) -> u8 {
        self.bpp
    }
    fn fb_ptr(&self) -> *mut u8 {
        self.fb_addr
    }
    fn fb_size(&self) -> usize {
        (self.pitch * self.height) as usize
    }
    fn pixel_format(&self) -> PixelFormat {
        self.format
    }
    fn set_mode(&mut self, _w: u32, _h: u32, _bpp: u8) -> bool {
        // GOP не поддерживает runtime mode-set через Limine
        false
    }
}
