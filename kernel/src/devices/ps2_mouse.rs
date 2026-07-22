// PS/2 Mouse Driver — драйвер мыши через контроллер PS/2.
//
// Протокол PS/2 мыши:
//   3-байтовый пакет: [код_кнопок | dx | dy]
//   Байт 0: [Yoverflow|Xoverflow|Ysign|Xsign|1|Middle|Right|Left]
//   Байт 1: dx (знаковое)
//   Байт 2: dy (знаковое)
//
// Управление через порты:
//   0x64 — командный порт контроллера PS/2
//   0x60 — порт данных

use core::sync::atomic::{AtomicU8, AtomicBool, AtomicI16, Ordering};

/// Состояние мыши
#[derive(Debug, Clone, Copy)]
pub struct MouseState {
    pub dx: i16,
    pub dy: i16,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

// Атомарное хранение состояния мыши (безопасно для ISR)
static MOUSE_DX: AtomicI16 = AtomicI16::new(0);
static MOUSE_DY: AtomicI16 = AtomicI16::new(0);
static MOUSE_LEFT: AtomicBool = AtomicBool::new(false);
static MOUSE_RIGHT: AtomicBool = AtomicBool::new(false);
static MOUSE_MIDDLE: AtomicBool = AtomicBool::new(false);

// Состояние парсера 3-байтного пакета
static PKT_INDEX: AtomicU8 = AtomicU8::new(0);
static PKT_BYTE0: AtomicU8 = AtomicU8::new(0);
static PKT_BYTE1: AtomicU8 = AtomicU8::new(0);

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
    val
}

/// Ждать готовности контроллера PS/2 (Input Buffer Empty)
unsafe fn wait_input_empty() {
    let mut timeout = 100_000u32;
    while inb(0x64) & 0x02 != 0 && timeout > 0 {
        timeout -= 1;
    }
}

/// Ждать данных в буфере (Output Buffer Full)
unsafe fn wait_output_full() -> bool {
    let mut timeout = 100_000u32;
    while inb(0x64) & 0x01 == 0 && timeout > 0 {
        timeout -= 1;
    }
    timeout > 0
}

/// Инициализация PS/2 мыши
pub fn init() {
    unsafe {
        // 1. Включаем второй порт PS/2 (команда 0xA8)
        wait_input_empty();
        outb(0x64, 0xA8);

        // 2. Читаем конфигурационный байт (команда 0x20)
        wait_input_empty();
        outb(0x64, 0x20);
        let _ = wait_output_full();
        let config = inb(0x60);

        // 3. Устанавливаем бит 1 (IRQ12) и бит 0 (IRQ1) в конфигурации
        let new_config = config | 0x03;
        wait_input_empty();
        outb(0x64, 0x60);
        wait_input_empty();
        outb(0x60, new_config);

        // 4. Включаем мышь (команда 0xD4 → 0x64, затем 0xF4 → 0x60)
        wait_input_empty();
        outb(0x64, 0xD4); // Следующий байт → второй порт PS/2
        wait_input_empty();
        outb(0x60, 0xF4); // Enable Data Reporting

        // Ждём ACK от мыши
        let _ = wait_output_full();
        let ack = inb(0x60);
        if ack != 0xFA {
            crate::println!("[MOUSE] WARNING: expected ACK (0xFA), got {:#x}", ack);
        }

        // 5. Размаскируем IRQ12 на PIC2 (IRQ4 на ведомом PIC)
        // PIC2_DATA = 0xA1, бит 4 = IRQ12
        let mask = inb(0xA1) & !0x04;
        outb(0xA1, mask);

        crate::println!("[MOUSE] PS/2 mouse initialized (IRQ12 enabled)");
    }
}

/// Обработчик прерывания IRQ12 — вызывается из ISR
pub fn handle_interrupt() {
    let byte = unsafe { inb(0x60) };
    let idx = PKT_INDEX.load(Ordering::Relaxed);

    match idx {
        0 => {
            // Первый байт — код кнопок (бит 3 всегда = 1)
            if byte & 0x08 == 0 {
                // Невалидный заголовок пакета — сброс парсера
                return;
            }
            PKT_BYTE0.store(byte, Ordering::Relaxed);
            PKT_INDEX.store(1, Ordering::Relaxed);
        }
        1 => {
            // Второй байт — dx
            PKT_BYTE1.store(byte, Ordering::Relaxed);
            PKT_INDEX.store(2, Ordering::Relaxed);
        }
        2 => {
            // Третий байт — dy, декодируем весь пакет
            let b0 = PKT_BYTE0.load(Ordering::Relaxed);
            let dx_raw = PKT_BYTE1.load(Ordering::Relaxed);
            let dy_raw = byte;

            // Кнопки
            MOUSE_LEFT.store(b0 & 0x01 != 0, Ordering::Relaxed);
            MOUSE_RIGHT.store(b0 & 0x02 != 0, Ordering::Relaxed);
            MOUSE_MIDDLE.store(b0 & 0x04 != 0, Ordering::Relaxed);

            // Накапливаем dx/dy
            let dx_val = dx_raw as i8 as i16;
            let dy_val = -(dy_raw as i16); // Инвертируем Y (экранные координаты)
            MOUSE_DX.fetch_add(dx_val, Ordering::Relaxed);
            MOUSE_DY.fetch_add(dy_val, Ordering::Relaxed);

            PKT_INDEX.store(0, Ordering::Relaxed);
        }
        _ => {
            // Сброс при неизвестном состоянии
            PKT_INDEX.store(0, Ordering::Relaxed);
        }
    }
}

/// Получить текущее состояние мыши (дельты сбрасываются после чтения)
pub fn get_state() -> MouseState {
    let dx = MOUSE_DX.swap(0, Ordering::Relaxed);
    let dy = MOUSE_DY.swap(0, Ordering::Relaxed);
    let left = MOUSE_LEFT.load(Ordering::Relaxed);
    let right = MOUSE_RIGHT.load(Ordering::Relaxed);
    let middle = MOUSE_MIDDLE.load(Ordering::Relaxed);

    MouseState { dx, dy, left_button: left, right_button: right, middle_button: middle }
}
