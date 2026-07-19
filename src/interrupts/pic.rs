use crate::println;
use core::arch::asm;

// Порты PIC
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

// Команды PIC
const PIC_EOI: u8 = 0x20;  // End Of Interrupt
const ICW1_INIT: u8 = 0x11;
const ICW4_8086: u8 = 0x01;

// Смещение векторов прерываний
pub const PIC1_OFFSET: u8 = 32;
pub const PIC2_OFFSET: u8 = 40;

#[inline]
unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn io_wait() {
    outb(0x80, 0);
}

pub fn init() {
    unsafe {
        println!("[PIC] Remapping IRQs to vectors 32-47...");
        
        let _mask1 = inb(PIC1_DATA);
        let _mask2 = inb(PIC2_DATA);
        
        outb(PIC1_COMMAND, ICW1_INIT);
        io_wait();
        outb(PIC2_COMMAND, ICW1_INIT);
        io_wait();
        
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();
        
        outb(PIC1_DATA, 4);
        io_wait();
        outb(PIC2_DATA, 2);
        io_wait();
        
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();
        
        // Разрешаем только IRQ0 (таймер) и IRQ1 (клавиатура),
        // остальные маскируем. Бит = 1 означает "замаскировано".
        outb(PIC1_DATA, 0b1111_1100);
        outb(PIC2_DATA, 0xFF);
        
        println!("[PIC] PIC initialized. IRQ0 (timer) + IRQ1 (keyboard) unmasked.");
    }
}

pub unsafe fn eoi(irq: u8) {
    if irq >= 8 {
        outb(PIC2_COMMAND, PIC_EOI);
    }
    outb(PIC1_COMMAND, PIC_EOI);
}

/// Замаскировать все IRQ на PIC (для использования APIC)
pub unsafe fn mask_all() {
    outb(PIC1_DATA, 0xFF); // Маскируем все 8 IRQ на мастер PIC
    outb(PIC2_DATA, 0xFF); // Маскируем все 8 IRQ на ведомый PIC
}