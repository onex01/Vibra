// APIC — полная замена PIC.
//
// LAPIC: локальный APIC для каждого ядра (MMIO 0xFEE00000)
// IO APIC: маршрутизация внешних IRQ
// LAPIC timer: замена PIT timer

use crate::println;

// LAPIC Registers (MMIO offset от базы)
const LAPIC_BASE: u64 = 0xFEE00000;
const LAPIC_ID: u64 = 0x020;
const LAPIC_TPR: u64 = 0x080;     // Task Priority Register
const LAPIC_EOI: u64 = 0x0B0;     // End Of Interrupt
const LAPIC_SVR: u64 = 0x0F0;     // Spurious Vector Register
const LAPIC_ICR_LOW: u64 = 0x300; // Interrupt Command Register
const LAPIC_LVT_TIMER: u64 = 0x320;
const LAPIC_TIMER_INIT: u64 = 0x380;
const LAPIC_TIMER_DIV: u64 = 0x3E0;

// IO APIC Registers
const IOAPIC_BASE: u64 = 0xFEC00000;
const IOAPIC_ID_REG: u64 = 0x00;
const IOAPIC_VER_REG: u64 = 0x01;
const IOAPIC_REDIRECTION: u64 = 0x10;

// MSR
const IA32_APIC_BASE: u32 = 0x1B;

// Векторы прерываний (выше 47 чтобы не конфликтовать с PIC-ремапом)
const LAPIC_TIMER_VECTOR: u8 = 48;  // LAPIC таймер

/// MMIO read/write
unsafe fn lapic_read(reg: u64) -> u32 {
    core::ptr::read_volatile((LAPIC_BASE + reg) as *const u32)
}
unsafe fn lapic_write(reg: u64, val: u32) {
    core::ptr::write_volatile((LAPIC_BASE + reg) as *mut u32, val);
}
unsafe fn ioapic_read(reg: u64) -> u32 {
    let base = IOAPIC_BASE as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::read_volatile(base.add(4))
}
unsafe fn ioapic_write(reg: u64, val: u32) {
    let base = IOAPIC_BASE as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::write_volatile(base.add(4), val);
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi);
    ((hi as u64) << 32) | (lo as u64)
}
unsafe fn wrmsr(msr: u32, val: u64) {
    core::arch::asm!("wrmsr", in("ecx") msr, in("eax") val as u32, in("edx") (val >> 32) as u32);
}

/// Проверить наличие APIC через CPUID
pub fn has_apic() -> bool {
    unsafe {
        let mut edx: u32;
        core::arch::asm!("cpuid", in("eax") 1u32, lateout("edx") edx, options(nomem));
        edx & (1 << 9) != 0
    }
}

/// EOI в LAPIC
pub fn eoi() {
    unsafe { lapic_write(LAPIC_EOI, 0); }
}

/// Инициализация LAPIC
unsafe fn init_lapic() {
    // Включаем APIC
    let mut apic_base = rdmsr(IA32_APIC_BASE);
    apic_base |= 1 << 11; // APIC Enable
    wrmsr(IA32_APIC_BASE, apic_base);

    // Spurious Vector: включаем APIC (бит 8) + spurious vector = 0xFF
    lapic_write(LAPIC_SVR, 0x1FF);

    // Все прерывания разрешены (TPR = 0)
    lapic_write(LAPIC_TPR, 0);

    // LINT0/LINT1 — отключаем (APIC сам обрабатывает)
    lapic_write(0x350, 0x00000100); // LINT0: Disabled
    lapic_write(0x360, 0x00000100); // LINT1: Disabled

    println!("[APIC] LAPIC initialized");
}

/// Инициализация LAPIC таймера (periodic, 100 Hz)
pub unsafe fn init_lapic_timer() {
    // Делитель 16 (0x0B), periodic mode (бит 12)
    lapic_write(LAPIC_TIMER_DIV, 0x0B);

    // LVT Timer: вектор LAPIC_TIMER_VECTOR, periodic
    lapic_write(LAPIC_LVT_TIMER, (LAPIC_TIMER_VECTOR as u32) | (1 << 12));

    // Initial Count: 1193182 / 16 / 100 = 7457 ticks for ~10ms at 100Hz
    // Формула: bus_freq / divider / desired_hz
    // При стандартной частоте 1193182 Hz и делителе 16: 1193182 / 16 / 100 ≈ 7457
    lapic_write(LAPIC_TIMER_INIT, 7457);

    println!("[APIC] LAPIC timer: 100 Hz, vector {}", LAPIC_TIMER_VECTOR);
}

/// Инициализация IO APIC
unsafe fn init_ioapic() {
    let ioapic_id = ioapic_read(IOAPIC_ID_REG) >> 24;
    let ioapic_ver = ioapic_read(IOAPIC_VER_REG);
    println!("[APIC] IO APIC id={}, ver={}", ioapic_id, ioapic_ver & 0xFF);

    // Маскируем все IRQ в IO APIC (bit 16 = mask)
    for i in 0..24 {
        ioapic_write(IOAPIC_REDIRECTION + i * 2, 1 << 16); // mask
    }

    // IRQ0 (PIT/таймер) → вектор 32
    ioapic_redirect(0, 32, 0);

    // IRQ1 (клавиатура) → вектор 33
    ioapic_redirect(1, 33, 0);

    // IRQ2 (каскад) → замаскирован
    // IRQ4 (serial) → вектор 36
    ioapic_redirect(4, 36, 0);

    println!("[APIC] IO APIC: IRQ0→32(tmr), IRQ1→33(kbd), IRQ4→36(serial)");
}

/// Настроить маршрутизацию IRQ
unsafe fn ioapic_redirect(irq: u32, vector: u32, destination: u32) {
    let low = vector & 0xFF;
    let high = (destination & 0xFF) << 24;
    let irq = irq as u64;
    ioapic_write(IOAPIC_REDIRECTION + irq * 2, low);
    ioapic_write(IOAPIC_REDIRECTION + irq * 2 + 1, high);
}

/// Полная инициализация APIC (замена PIC)
pub fn init() {
    if !has_apic() {
        println!("[APIC] No APIC detected, PIC will be used");
        return;
    }

    println!("[APIC] APIC detected, initializing...");

    unsafe {
        init_lapic();
        init_ioapic();
        init_lapic_timer();
    }

    // Полностью маскируем PIC — APIC берёт на себя
    unsafe {
        crate::interrupts::pic::mask_all();
    }

    println!("[APIC] APIC fully initialized (PIC masked)");
}

/// Получить вектор LAPIC таймера (для IDT)
pub const fn timer_vector() -> u8 {
    LAPIC_TIMER_VECTOR
}
