/// PCI VGA backend — legacy VGA для старого железа
///
/// Управление через I/O порты VGA:
///   0x3C4/0x3C5 — Sequencer
///   0x3D4/0x3D5 — CRT Controller (color)
///   0x3CE/0x3CF — Graphics Controller
///   0x3C0/0x3C1 — Attribute Controller
///   0x3C8/0x3C9 — DAC palette
///   0x3DA — Input Status 1 (read)
use super::{DisplayBackend, PixelFormat};

// VGA I/O порты (color mode)
const VGA_SEQ_ADDR: u16 = 0x3C4;
const VGA_SEQ_DATA: u16 = 0x3C5;
const VGA_CRTC_ADDR: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;
const VGA_GC_ADDR: u16 = 0x3CE;
const VGA_GC_DATA: u16 = 0x3CF;
const VGA_AC_ADDR: u16 = 0x3C0;
const VGA_DAC_WRITE: u16 = 0x3C8;
const VGA_DAC_DATA: u16 = 0x3C9;
const VGA_INPUT_STATUS: u16 = 0x3DA;

// VGA режимы
const VGA_MODE_12H: u16 = 0x0012; // 640x480x16цветов (4bpp planar)
const VGA_MODE_13H: u16 = 0x0013; // 320x200x256цветов (8bpp packed)

// VGA MEMORY base
const VGA_MEM_BASE: u64 = 0xA0000;

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

unsafe fn vga_write_seq(reg: u8, val: u8) {
    outb(VGA_SEQ_ADDR, reg);
    outb(VGA_SEQ_DATA, val);
}

unsafe fn vga_write_crtc(reg: u8, val: u8) {
    outb(VGA_CRTC_ADDR, reg);
    outb(VGA_CRTC_DATA, val);
}

unsafe fn vga_write_gc(reg: u8, val: u8) {
    outb(VGA_GC_ADDR, reg);
    outb(VGA_GC_DATA, val);
}

unsafe fn vga_wait_retrace() {
    // Ждём начало вертикального обратного хода
    while inb(VGA_INPUT_STATUS) & 0x08 != 0 {}
    while inb(VGA_INPUT_STATUS) & 0x08 == 0 {}
}

pub struct PciVgaBackend {
    fb_addr: *mut u8,   // VGA memory at 0xA0000 (через HHDM)
    io_base: u16,        // 0x3D0 для color VGA
    width: u32,
    height: u32,
    pitch: u32,
    current_mode: u16,
    has_vesa: bool,
}

unsafe impl Send for PciVgaBackend {}
unsafe impl Sync for PciVgaBackend {}

impl PciVgaBackend {
    /// Создать VGA бэкенд из PCI устройства (class 0x03, subclass 0x00)
    pub fn new(hhdm: u64) -> Self {
        let fb_addr = (hhdm + VGA_MEM_BASE) as *mut u8;
        Self {
            fb_addr,
            io_base: 0x3D0,
            width: 640,
            height: 480,
            pitch: 80, // 640/8 = 80 байт на строку (planar mode)
            current_mode: VGA_MODE_12H,
            has_vesa: false,
        }
    }

    /// Установить VGA видео режим через программирование регистров
    pub fn set_vga_mode(&mut self, mode: u16) -> bool {
        unsafe {
            // Отключаем дисплей: Sequencer reg 1, bit 5 (Screen Off)
            vga_write_seq(0x01, 0x20);

            // Ждём обратного хода
            vga_wait_retrace();

            match mode {
                VGA_MODE_12H => {
                    // 640x480x4bpp planar
                    self.width = 640;
                    self.height = 480;
                    self.pitch = 80;

                    // Sequencer: 8/9 dot clock, устанавливаем бит 4 в reg 1 (dot clock)
                    vga_write_seq(0x00, 0x03); // Reset
                    vga_write_seq(0x01, 0x01); // Clocking mode
                    vga_write_seq(0x02, 0x0F); // Map mask: все 4 плоскости
                    vga_write_seq(0x03, 0x00); // Character select
                    vga_write_seq(0x04, 0x0E); // Memory mode: odd/even, extents

                    // CRTC: тайминги для 640x480
                    vga_write_crtc(0x00, 0x57); // Horizontal Total
                    vga_write_crtc(0x01, 0x4F); // Horizontal Display End
                    vga_write_crtc(0x02, 0x50); // Start Horizontal Blanking
                    vga_write_crtc(0x03, 0x82); // End Horizontal Blanking
                    vga_write_crtc(0x04, 0x55); // Start Horizontal Retrace
                    vga_write_crtc(0x05, 0x81); // End Horizontal Retrace
                    vga_write_crtc(0x06, 0x0B); // Vertical Total
                    vga_write_crtc(0x07, 0x3E); // Overflow
                    vga_write_crtc(0x08, 0x00); // Preset Row Scan
                    vga_write_crtc(0x09, 0x40); // Max Scan Line
                    vga_write_crtc(0x0A, 0x00); // Cursor Start
                    vga_write_crtc(0x0B, 0x00); // Cursor End
                    vga_write_crtc(0x0C, 0x00); // Start Address High
                    vga_write_crtc(0x0D, 0x00); // Start Address Low
                    vga_write_crtc(0x0E, 0x00); // Cursor Address High
                    vga_write_crtc(0x0F, 0x00); // Cursor Address Low
                    vga_write_crtc(0x10, 0xE9); // Vertical Retrace Start
                    vga_write_crtc(0x11, 0x8B); // Vertical Retrace End
                    vga_write_crtc(0x12, 0xDF); // Vertical Display End
                    vga_write_crtc(0x13, 0x28); // Offset
                    vga_write_crtc(0x14, 0x00); // Underline Location
                    vga_write_crtc(0x15, 0xE7); // Start Vertical Blanking
                    vga_write_crtc(0x16, 0x04); // End Vertical Blanking
                    vga_write_crtc(0x17, 0xE3); // CRTC Mode Control
                    vga_write_crtc(0x18, 0xFF); // Line Compare

                    // Graphics Controller
                    vga_write_gc(0x00, 0x00); // Set/Reset
                    vga_write_gc(0x01, 0x00); // Enable Set/Reset
                    vga_write_gc(0x02, 0x00); // Color Compare
                    vga_write_gc(0x03, 0x00); // Data Rotate
                    vga_write_gc(0x04, 0x00); // Read Map Select
                    vga_write_gc(0x05, 0x00); // Graphics Mode
                    vga_write_gc(0x06, 0x05); // Miscellaneous: A0000-AFFFF, mode 0x05
                    vga_write_gc(0x07, 0x0F); // Color Don't Care
                    vga_write_gc(0x08, 0xFF); // Bit Mask

                    self.current_mode = mode;
                }
                VGA_MODE_13H => {
                    // 320x200x256 (mode 13h) — packed pixel
                    self.width = 320;
                    self.height = 200;
                    self.pitch = 320; // 320 байт на строку (1 байт/пиксель)

                    // Sequencer
                    vga_write_seq(0x00, 0x03); // Reset
                    vga_write_seq(0x01, 0x01); // Clocking mode
                    vga_write_seq(0x02, 0x0F); // Map mask: все плоскости
                    vga_write_seq(0x03, 0x00); // Character select
                    vga_write_seq(0x04, 0x0E); // Memory mode

                    // CRTC: тайминги для 320x200
                    vga_write_crtc(0x00, 0x57);
                    vga_write_crtc(0x01, 0x4F);
                    vga_write_crtc(0x02, 0x50);
                    vga_write_crtc(0x03, 0x82);
                    vga_write_crtc(0x04, 0x54);
                    vga_write_crtc(0x05, 0x80);
                    vga_write_crtc(0x06, 0x0D);
                    vga_write_crtc(0x07, 0x3E);
                    vga_write_crtc(0x08, 0x00);
                    vga_write_crtc(0x09, 0x41);
                    vga_write_crtc(0x0A, 0x00);
                    vga_write_crtc(0x0B, 0x00);
                    vga_write_crtc(0x0C, 0x00);
                    vga_write_crtc(0x0D, 0x00);
                    vga_write_crtc(0x0E, 0x00);
                    vga_write_crtc(0x0F, 0x00);
                    vga_write_crtc(0x10, 0x9C);
                    vga_write_crtc(0x11, 0x8E);
                    vga_write_crtc(0x12, 0x8F);
                    vga_write_crtc(0x13, 0x28);
                    vga_write_crtc(0x14, 0x00);
                    vga_write_crtc(0x15, 0x96);
                    vga_write_crtc(0x16, 0xB9);
                    vga_write_crtc(0x17, 0xE3);
                    vga_write_crtc(0x18, 0xFF);

                    // Graphics Controller
                    vga_write_gc(0x00, 0x00);
                    vga_write_gc(0x01, 0x00);
                    vga_write_gc(0x02, 0x00);
                    vga_write_gc(0x03, 0x00);
                    vga_write_gc(0x04, 0x00);
                    vga_write_gc(0x05, 0x40); // Graphics mode: packed pixel
                    vga_write_gc(0x06, 0x05);
                    vga_write_gc(0x07, 0x0F);
                    vga_write_gc(0x08, 0xFF);

                    self.current_mode = mode;
                }
                _ => {
                    // Неизвестный режим
                    crate::println!("[VGA] Неизвестный режим: {:#x}", mode);
                    return false;
                }
            }

            // Включаем дисплей: снимаем Screen Off
            vga_write_seq(0x01, 0x01);
        }
        true
    }

    /// Очистить VGA framebuffer
    pub fn clear(&self) {
        unsafe {
            let total = (self.pitch * self.height) as usize;
            let slice = core::slice::from_raw_parts_mut(self.fb_addr, total);
            slice.fill(0);
        }
    }
}

impl DisplayBackend for PciVgaBackend {
    fn name(&self) -> &str {
        "PCI VGA"
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
        match self.current_mode {
            VGA_MODE_13H => 8,
            VGA_MODE_12H => 4,
            _ => 8,
        }
    }
    fn fb_ptr(&self) -> *mut u8 {
        self.fb_addr
    }
    fn fb_size(&self) -> usize {
        (self.pitch * self.height) as usize
    }
    fn pixel_format(&self) -> PixelFormat {
        match self.current_mode {
            VGA_MODE_13H => PixelFormat::Indexed8,
            VGA_MODE_12H => PixelFormat::Indexed8,
            _ => PixelFormat::Text,
        }
    }
    fn set_mode(&mut self, width: u32, height: u32, _bpp: u8) -> bool {
        // Попытка подобрать режим
        let mode = if width >= 640 && height >= 480 {
            VGA_MODE_12H
        } else if width >= 320 && height >= 200 {
            VGA_MODE_13H
        } else {
            return false;
        };
        self.set_vga_mode(mode)
    }
    fn flush(&mut self) {
        // VGA framebuffer — memory-mapped, flush не требуется
    }
}
