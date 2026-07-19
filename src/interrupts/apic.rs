// APIC — Advanced Programmable Interrupt Controller
//
// Заменяет 8259 PIC на現代ен APIC.
// LAPIC: Local APIC (для каждого ядра)
// IO APIC: маршрутизация внешних IRQ

use crate::println;

// LAPIC Registers (MMIO)
const LAPIC_BASE: u64 = 0xFEE00000;
const LAPIC_ID: u64 = 0x020;
const LAPIC_TPR: u64 = 0x080;    // Task Priority Register
const LAPIC_EOI: u64 = 0x0B0;    // End Of Interrupt
const LAPIC_SVR: u64 = 0x0F0;    // Spurious Vector Register
const LAPIC_ICR_LOW: u64 = 0x300; // Interrupt Command Register
const LAPIC_ICR_HIGH: u64 = 0x310;
const LAPIC_LVT_TIMER: u64 = 0x320;
const LAPIC_LVT_LINT0: u64 = 0x350;
const LAPIC_LVT_LINT1: u64 = 0x360;
const LAPIC_TIMER_INIT: u64 = 0x380;
const LAPIC_TIMER_CURRENT: u64 = 0x390;
const LAPIC_TIMER_DIV: u64 = 0x3E0;

// IO APIC Registers
const IOAPIC_BASE: u64 = 0xFEC00000;
const IOAPIC_ID: u64 = 0x00;
const IOAPIC_VER: u64 = 0x01;
const IOAPIC_REDIRECTION: u64 = 0x10;

// MSR
const IA32_APIC_BASE: u32 = 0x1B;

/// Прочитать 32-битное значение из MMIO
unsafe fn lapic_read(reg: u64) -> u32 {
    let ptr = (LAPIC_BASE + reg) as *const u32;
    core::ptr::read_volatile(ptr)
}

/// Записать 32-битное значение в MMIO
unsafe fn lapic_write(reg: u64, val: u32) {
    let ptr = (LAPIC_BASE + reg) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

/// Прочитать из IO APIC
unsafe fn ioapic_read(reg: u64) -> u32 {
    let base = IOAPIC_BASE as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::read_volatile(base.add(4))
}

/// Записать в IO APIC
unsafe fn ioapic_write(reg: u64, val: u32) {
    let base = IOAPIC_BASE as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::write_volatile(base.add(4), val);
}

/// Прочитать MSR
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high);
    ((high as u64) << 32) | (low as u64)
}

/// Записать MSR
unsafe fn wrmsr(msr: u32, val: u64) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    core::arch::asm!("wrmsr", in("ecx") msr, in("eax") low, in("edx") high);
}

/// Проверить наличие APIC через CPUID
pub fn has_apic() -> bool {
    unsafe {
        let mut edx: u32;
        core::arch::asm!(
            "cpuid",
            in("eax") 1u32,
            lateout("edx") edx,
            options(nostack),
        );
        edx & (1 << 9) != 0 // Bit 9 = APIC
    }
}

/// Инициализация LAPIC
unsafe fn init_lapic() {
    // Включаем APIC через MSR
    let mut apic_base = rdmsr(IA32_APIC_BASE);
    apic_base |= 1 << 11; // APIC Global Enable
    wrmsr(IA32_APIC_BASE, apic_base);

    // Spurious Vector Register: включаем APIC (бит 8) + spurious vector = 0xFF
    lapic_write(LAPIC_SVR, 0x1FF);

    // Запрещаем все прерывания через TPR (приоритет = 0 = все разрешены)
    lapic_write(LAPIC_TPR, 0);

    // LINT0/LINT1 = ExtINT / NMI (для совместимости с PIC)
    lapic_write(LAPIC_LVT_LINT0, 0x00000870); // ExtINT
    lapic_write(LAPIC_LVT_LINT1, 0x00000400); // NMI

    // Таймер: one-shot, вектор 32
    lapic_write(LAPIC_LVT_TIMER, 32);
    lapic_write(LAPIC_TIMER_DIV, 0x0B); // Divide by 16

    println!("[APIC] LAPIC initialized at {:#x}", LAPIC_BASE);
}

/// Калибровка LAPIC таймера через PIT
unsafe fn calibrate_timer() -> u32 {
    // Используем PIT channel 2 для калибровки
    // Устанавливаем LAPIC таймер на максимальное значение
    lapic_write(LAPIC_TIMER_INIT, 0xFFFFFFFF);

    // Ждём ~10мс через PIT channel 2 (порт 0x61)
    // Простой busy-wait
    for _ in 0..100_000 {
        core::hint::spin_loop();
    }

    // Читаем оставшееся значение
    let current = lapic_read(LAPIC_TIMER_CURRENT);
    let ticks_per_10ms = 0xFFFFFFFF - current;
    let ticks_per_sec = ticks_per_10ms * 100;

    println!("[APIC] Timer calibrated: {} ticks/sec", ticks_per_sec);
    ticks_per_sec as u32
}

/// Инициализация IO APIC
unsafe fn init_ioapic() {
    // Читаем ID IO APIC
    let ioapic_id = ioapic_read(IOAPIC_ID);
    println!("[APIC] IO APIC ID: {}", (ioapic_id >> 24) & 0xF);

    // Читаем версию
    let ioapic_ver = ioapic_read(IOAPIC_VER);
    let max_redirect = (ioapic_ver >> 16) & 0xFF;
    println!("[APIC] IO APIC version: {}, max redirections: {}", ioapic_ver & 0xFF, max_redirect);

    // Маршрутизация IRQ → векторы:
    // IRQ1 (клавиатура) → вектор 33
    // IRQ0 (таймер) → вектор 32
    // Формат: [phys_dest][logical_dest][delivery_mode][dest_mode][polarity][trigger][mask][vector]

    // IRQ0 → вектор 32 (таймер)
    ioapic_redirect(0, 32, 0); // delivery=physical, dest=0 (BSP)

    // IRQ1 → вектор 33 (клавиатура)
    ioapic_redirect(1, 33, 0);

    println!("[APIC] IO APIC initialized (IRQ0→32, IRQ1→33)");
}

/// Настроить маршрутизацию IRQ
unsafe fn ioapic_redirect(irq: u32, vector: u32, destination: u32) {
    let redirect_low = (vector & 0xFF) as u32;  // Vector
    let redirect_high = (destination & 0xFF) << 24; // Destination CPU

    let irq = irq as u64;
    ioapic_write(IOAPIC_REDIRECTION + irq * 2, redirect_low);
    ioapic_write(IOAPIC_REDIRECTION + irq * 2 + 1, redirect_high);
}

/// Отправить EOI в LAPIC
pub fn eoi() {
    unsafe {
        lapic_write(LAPIC_EOI, 0);
    }
}

/// Полная инициализация APIC
pub fn init() {
    if !has_apic() {
        println!("[APIC] No APIC detected, falling back to PIC");
        return;
    }

    unsafe {
        init_lapic();
        calibrate_timer();
        init_ioapic();
    }

    println!("[APIC] APIC initialization complete");
}
