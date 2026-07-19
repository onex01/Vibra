// APIC — detection and basic initialization.
//
// Phase 2: только детект и LAPIC base.
// Полная интеграция IO APIC + LAPIC timer будет позже.

use crate::println;

const IA32_APIC_BASE: u32 = 0x1B;
const LAPIC_BASE: u64 = 0xFEE00000;
const IOAPIC_BASE: u64 = 0xFEC00000;

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

/// Прочитать 32-битное значение из MMIO
unsafe fn mmio_read(base: u64, reg: u64) -> u32 {
    let ptr = (base + reg) as *const u32;
    core::ptr::read_volatile(ptr)
}

/// Записать 32-битное значение в MMIO
unsafe fn mmio_write(base: u64, reg: u64, val: u32) {
    let ptr = (base + reg) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

/// Проверить наличие APIC через CPUID
pub fn has_apic() -> bool {
    unsafe {
        let mut edx: u32;
        core::arch::asm!(
            "cpuid",
            in("eax") 1u32,
            lateout("edx") edx,
            options(nomem),
        );
        edx & (1 << 9) != 0
    }
}

/// Прочитать LAPIC ID
pub fn lapic_id() -> u32 {
    unsafe { mmio_read(LAPIC_BASE, 0x020) >> 24 }
}

/// Прочитать IO APIC ID
pub fn ioapic_id() -> u32 {
    unsafe {
        mmio_write(IOAPIC_BASE, 0x00, 0x00); // IOAPICID
        mmio_read(IOAPIC_BASE, 0x10) >> 24
    }
}

/// Прочитать версию IO APIC
pub fn ioapic_version() -> u32 {
    unsafe {
        mmio_write(IOAPIC_BASE, 0x00, 0x01); // IOAPICVER
        mmio_read(IOAPIC_BASE, 0x10) & 0xFF
    }
}

/// Инициализация APIC (детект + базовая информация)
pub fn init() {
    if !has_apic() {
        println!("[APIC] No APIC detected");
        return;
    }

    println!("[APIC] APIC detected:");

    // Читаем LAPIC ID
    let lapic_id_val = lapic_id();
    println!("[APIC]   LAPIC ID: {}", lapic_id_val);

    // Читаем IO APIC info
    let ioapic_id_val = ioapic_id();
    let ioapic_ver = ioapic_version();
    println!("[APIC]   IO APIC ID: {}, version: {}", ioapic_id_val, ioapic_ver);

    // Включаем APIC через MSR
    unsafe {
        let mut apic_base = rdmsr(IA32_APIC_BASE);
        if apic_base & (1 << 11) == 0 {
            apic_base |= 1 << 11; // APIC Global Enable
            wrmsr(IA32_APIC_BASE, apic_base);
            println!("[APIC]   APIC Global Enable set");
        }
    }

    println!("[APIC] APIC initialized (PIC remains primary for now)");
}
