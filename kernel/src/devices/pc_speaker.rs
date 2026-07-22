// PC Speaker — эмуляция динамика через PIT Channel 2.
//
// Порт 0x61: бит 0 = таймер 2 gated, бит 1 = speaker data
// Порт 0x42: PIT Channel 2 data
// Порт 0x43: PIT command

use core::arch::asm;

const PIT_CHANNEL2_DATA: u16 = 0x42;
const PIT_COMMAND: u16 = 0x43;
const SPEAKER_PORT: u16 = 0x61;

#[inline]
unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
    val
}

/// Включить звук на заданной частоте (Гц)
pub fn beep(frequency: u32) {
    if frequency == 0 {
        silent();
        return;
    }

    let divisor = 1193180 / frequency;

    unsafe {
        // Настраиваем PIT Channel 2
        outb(PIT_COMMAND, 0xB6); // Channel 2, lobyte/hibyte, square wave
        outb(PIT_CHANNEL2_DATA, divisor as u8);
        outb(PIT_CHANNEL2_DATA, (divisor >> 8) as u8);

        // Включаем speaker
        let tmp = inb(SPEAKER_PORT);
        if tmp | 3 != tmp {
            outb(SPEAKER_PORT, tmp | 3);
        }
    }
}

/// Выключить звук
pub fn silent() {
    unsafe {
        let tmp = inb(SPEAKER_PORT);
        outb(SPEAKER_PORT, tmp & !3);
    }
}

/// Короткий звуковой сигнал
pub fn beep_short() {
    beep(1000);
    // Примерная задержка ~100ms через busy-wait PIT
    // В реальной ОС использовал бы таймер
    for _ in 0..5_000_000u32 {
        core::hint::spin_loop();
    }
    silent();
}
