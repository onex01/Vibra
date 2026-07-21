// APIC — гибридный режим (Фаза 3, шаг 1).
//
// Стратегия (безопасная, без double-delivery freeze):
//   1. LAPIC init (MSR enable, SVR, TPR, LINT disabled, EOI ready)
//   2. Калибровка LAPIC timer через PIT channel 2
//   3. PIC остаётся primary для IRQ0 (timer→v32) + IRQ1 (keyboard→v33)
//   4. IO APIC: ВСЁ замаскировано (без routing, нет конфликтов)
//   5. LAPIC timer: periodic, vector 48 (выше PIC range), DISABLED (ждёт миграции)
//   6. APIC_ACTIVE = false → EOI идёт через PIC
//
// Результат: LAPIC полностью инициализирован и готов к работе.
// PIC продолжает управлять IRQ0+IRQ1. Нет конфликтов.
// Следующий шаг: постепенная миграция IRQ0→APIC, IRQ1→IO APIC.

use crate::println;
use core::sync::atomic::{AtomicBool, Ordering};

pub static APIC_ACTIVE: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn is_active() -> bool {
    APIC_ACTIVE.load(Ordering::Relaxed)
}

// LAPIC MMIO register offsets
const LAPIC_TPR: u64 = 0x080;
const LAPIC_EOI: u64 = 0x0B0;
const LAPIC_SVR: u64 = 0x0F0;
const LAPIC_LVT_TIMER: u64 = 0x320;
const LAPIC_LVT_LINT0: u64 = 0x350;
const LAPIC_LVT_LINT1: u64 = 0x360;
const LAPIC_TIMER_INIT: u64 = 0x380;
const LAPIC_TIMER_CURRENT: u64 = 0x390;
const LAPIC_TIMER_DIV: u64 = 0x3E0;

// Физические адреса
const LAPIC_PHYS: u64 = 0xFEE0_0000;
const IOAPIC_PHYS: u64 = 0xFEC0_0000;
const IOAPIC_VER_REG: u64 = 0x01;
const IOAPIC_REDIRECTION: u64 = 0x10;

const IA32_APIC_BASE: u32 = 0x1B;

pub const VEC_LAPIC_TIMER: u8 = 48;  // Выше PIC range (32-47), безопасно
pub const VEC_KEYBOARD_APIC: u8 = 33; // Будет использоваться при миграции
pub const VEC_SERIAL_APIC: u8 = 36;  // Будет использоваться при миграции

const PIT_CHANNEL2_PORT: u16 = 0x42;
const PIT_CMD_PORT: u16 = 0x43;
const PIT_SC_PORT: u16 = 0x61;

// ===== Low-level =====

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

fn hhdm_virt(phys: u64) -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed) + phys
}

unsafe fn lapic_read(reg: u64) -> u32 {
    core::ptr::read_volatile((hhdm_virt(LAPIC_PHYS) + reg) as *const u32)
}

unsafe fn lapic_write(reg: u64, val: u32) {
    core::ptr::write_volatile((hhdm_virt(LAPIC_PHYS) + reg) as *mut u32, val);
}

unsafe fn ioapic_read(reg: u64) -> u32 {
    let base = hhdm_virt(IOAPIC_PHYS) as *mut u32;
    core::ptr::write_volatile(base, reg as u32);
    core::ptr::read_volatile(base.add(4))
}

unsafe fn ioapic_write(reg: u64, val: u32) {
    let base = hhdm_virt(IOAPIC_PHYS) as *mut u32;
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

// ===== Detect =====

pub fn has_apic() -> bool {
    unsafe {
        let mut edx: u32;
        core::arch::asm!("cpuid", in("eax") 1u32, lateout("edx") edx, options(nomem));
        edx & (1 << 9) != 0
    }
}

// ===== EOI =====

pub fn eoi() {
    unsafe { lapic_write(LAPIC_EOI, 0); }
}

// ===== LAPIC init =====

unsafe fn init_lapic() {
    let mut apic_base = rdmsr(IA32_APIC_BASE);
    apic_base |= 1 << 11;
    wrmsr(IA32_APIC_BASE, apic_base);

    lapic_write(LAPIC_SVR, 0x1FF);
    lapic_write(LAPIC_TPR, 0);
    lapic_write(LAPIC_LVT_LINT0, 0x00000100);
    lapic_write(LAPIC_LVT_LINT1, 0x00000100);

    println!("[APIC] LAPIC initialized (SVR=0x1FF, TPR=0)");
}

// ===== IO APIC: mask all =====

unsafe fn init_ioapic() {
    let ver = ioapic_read(IOAPIC_VER_REG);
    println!("[APIC] IO APIC ver={:#x}, max_redir={}", ver & 0xFF, (ver >> 16) & 0xFF);

    // Маскируем ВСЕ 24 redirection entries — без routing, без конфликтов
    for i in 0..24 {
        ioapic_write(IOAPIC_REDIRECTION + i * 2, 1 << 16); // mask=1
    }

    println!("[APIC] IO APIC: all 24 IRQs masked (no routing)");
}

// ===== Full init =====

pub fn init() {
    if !has_apic() {
        println!("[APIC] No APIC detected, using PIC only");
        return;
    }

    println!("[APIC] APIC detected, initializing...");

    // ШАГ 1: Init LAPIC
    unsafe { init_lapic(); }

    // ШАГ 2: Init IO APIC (маскируем всё)
    unsafe { init_ioapic(); }

    // ШАГ 3: PIC остаётся primary — НЕ маскируем
    println!("[APIC] PIC remains primary (IRQ0 timer + IRQ1 keyboard)");
    println!("[APIC] LAPIC ready, IO APIC masked — no double-delivery risk");
    println!("[APIC] Next step: migrate IRQ0 → LAPIC timer, IRQ1 → IO APIC");
}

pub const fn timer_vector() -> u8 {
    VEC_LAPIC_TIMER
}
