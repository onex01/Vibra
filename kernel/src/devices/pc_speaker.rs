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

#[inline]
unsafe fn io_wait() {
    outb(0x80, 0);
}

/// Включить звук на заданной частоте (Гц)
pub fn beep(frequency: u32) {
    if frequency == 0 {
        silent();
        return;
    }

    // PIT: 1.193182 МГц. Делитель обязан быть в 1..=65535, иначе частота
    // уезжает в ультразвук/слышимый шум и динамик «молчит».
    let divisor = (1_193_180u32 / frequency.clamp(20, 20_000)).clamp(1, 65_535) as u16;

    unsafe {
        // Channel 2, lobyte/hibyte, mode 3 (square wave).
        outb(PIT_COMMAND, 0xB6);
        io_wait();
        outb(PIT_CHANNEL2_DATA, divisor as u8);
        io_wait();
        outb(PIT_CHANNEL2_DATA, (divisor >> 8) as u8);
        io_wait();

        // Включаем speaker: бит 0 = gate, бит 1 = data.
        let tmp = inb(SPEAKER_PORT);
        outb(SPEAKER_PORT, tmp | 0x03);
        io_wait();
    }
}

/// Выключить звук
pub fn silent() {
    unsafe {
        let tmp = inb(SPEAKER_PORT);
        outb(SPEAKER_PORT, tmp & !0x03);
        io_wait();
    }
}

/// Короткий звуковой сигнал.
pub fn beep_short() {
    beep(1000);
    let start = crate::timer::uptime_ms();
    while crate::timer::uptime_ms().wrapping_sub(start) < 100 {
        core::hint::spin_loop();
    }
    silent();
}
