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
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
const LAPIC_TIMER_INIT: u64 = 0x380;
const LAPIC_TIMER_CURRENT: u64 = 0x390;
const LAPIC_TIMER_DIV: u64 = 0x3E0;

// Физические адреса
const LAPIC_PHYS: u64 = 0xFEE0_0000;
const IOAPIC_PHYS: u64 = 0xFEC0_0000;
const IOAPIC_VER_REG: u64 = 0x01;
const IOAPIC_REDIRECTION: u64 = 0x10;

/// Реальный базовый адрес LAPIC (MADT может отличаться от 0xFEE00000).
static LAPIC_BASE_PHYS: AtomicU64 = AtomicU64::new(LAPIC_PHYS);

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
    let addr = hhdm_virt(LAPIC_BASE_PHYS.load(Ordering::Relaxed)) + reg;
    let val: u32;
    core::arch::asm!(
        "mov rax, [rdi]",
        out("rax") val,
        in("rdi") addr,
        options(nostack, nomem),
    );
    val
}

unsafe fn lapic_write(reg: u64, val: u32) {
    let addr = hhdm_virt(LAPIC_BASE_PHYS.load(Ordering::Relaxed)) + reg;
    core::arch::asm!(
        "mov [rdi], eax",
        in("rdi") addr,
        in("eax") val,
        options(nostack, nomem),
    );
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
    // __cpuid корректно описывает клобберы eax/ebx/ecx/edx; ручной asm
    // "cpuid" без lateout("ebx"/"ecx") ломал значение ebx у вызывающего кода.
    unsafe { core::arch::x86_64::__cpuid(1).edx & (1 << 9) != 0 }
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

    // LINT0/LINT1 НЕ трогаем. На части чипсетов выход 8259 (INTR) заведён
    // на LINT0 в режиме ExtINT, и маска LINT0 отрезала бы вообще все
    // PIC-прерывания (IRQ1 клавиатуры в том числе). Прошивка уже настроила
    // эти линии; LAPIC timer от них не зависит.

    println!("[APIC] LAPIC initialized (SVR=0x1FF, TPR=0)");
}

/// Калибровка LAPIC timer через PIT channel 2.
/// PIT channel 2: one-shot, count = PIT_FREQ/100 = 11932 (10мс).
/// Считаем LAPIC ticks за 10мс → вычисляем init для 100Hz.
unsafe fn calibrate_lapic_timer() -> u32 {
    // PIT Channel 2: one-shot mode (mode 0), lobyte/hibyte
    outb(0x43, 0xB0); // Channel 2, lobyte/hibyte, mode 0
    let pit_count: u16 = 11932; // 10ms at 1.193182 MHz
    outb(0x42, (pit_count & 0xFF) as u8);
    outb(0x42, (pit_count >> 8) as u8);

    // Disable PIT channel 2 gate
    let mut sc = inb(0x61);
    sc &= !0x01; // Gate 2 = 0 (disable counting)
    sc &= !0x20; // TMR2_OUT = 0
    outb(0x61, sc);

    // LAPIC timer: one-shot, max count
    lapic_write(LAPIC_LVT_TIMER, 48); // vector 48, one-shot
    lapic_write(LAPIC_TIMER_DIV, 0x03); // divide by 16
    lapic_write(LAPIC_TIMER_INIT, 0xFFFFFFFF);

    // Enable PIT gate → start counting
    sc = inb(0x61);
    sc |= 0x01; // Gate 2 = 1
    outb(0x61, sc);

    // Poll PIT TMR2_OUT (bit 5 of port 0x61)
    let mut timeout: u32 = 100_000;
    while inb(0x61) & 0x20 == 0 && timeout > 0 {
        timeout -= 1;
    }

    let remaining = lapic_read(LAPIC_TIMER_CURRENT);
    let ticks_10ms = 0xFFFFFFFFu32.wrapping_sub(remaining);

    // Init value для periodic 100Hz
    let init = ticks_10ms; // ticks за 10мс = ticks за один период 100Hz

    println!("[APIC] Calibration: {} LAPIC ticks/10ms, init={}", ticks_10ms, init);
    init
}

/// Запуск LAPIC timer в periodic mode
unsafe fn start_lapic_timer(init: u32) {
    lapic_write(LAPIC_TIMER_DIV, 0x03); // divide by 16
    lapic_write(LAPIC_LVT_TIMER, 48 | (1 << 12)); // vector 48, periodic (bit 12)
    lapic_write(LAPIC_TIMER_INIT, init);
    println!("[APIC] LAPIC timer: periodic ~100Hz, vector 48, init={}", init);
}

/// Количество активных IRQ в IO APIC
pub fn ioapic_mask_irq(irq: u32, masked: bool) {
    let low = if masked { 1u32 << 16 } else { 0u32 };
    unsafe { ioapic_write(IOAPIC_REDIRECTION + (irq as u64) * 2, low); }
}

/// Настроить IO APIC redirect: IRQ → vector, destination LAPIC
pub fn ioapic_redirect(irq: u32, vector: u8, destination: u8) {
    let low = vector as u32;
    let high = (destination as u32) << 24;
    unsafe {
        ioapic_write(IOAPIC_REDIRECTION + (irq as u64) * 2, low);
        ioapic_write(IOAPIC_REDIRECTION + (irq as u64) * 2 + 1, high);
    }
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

/// Legacy-путь миграции PIC → LAPIC (сейчас НЕ вызывается из boot).
///
/// ВНИМАНИЕ: если вызвать его вместе с `timer::post_init()`, получатся два
/// источника тиков (PIC IRQ0 и LAPIC vector 48), и `TICKS` будет идти вдвое
/// быстрее. Рабочий fallback для «мёртвого» IRQ0 — `start_periodic_timer()`.
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

    // ШАГ 3: Калибровка LAPIC timer через PIT channel 2
    let lapic_init = unsafe { calibrate_lapic_timer() };

    // ШАГ 4: Запуск LAPIC timer (periodic, vector 48)
    unsafe { start_lapic_timer(lapic_init); }

    // ШАГ 5: PIC остаётся primary — НЕ маскируем (PIO keyboard/timer через PIC)
    // ШАГ 6: APIC_ACTIVE = false — EOI через PIC
    println!("[APIC] PIC remains primary (IRQ0 timer + IRQ1 keyboard)");
    println!("[APIC] LAPIC timer calibrated and started (vector 48, ~100Hz)");
    println!("[APIC] Ready for IRQ migration when stable");
}

/// Запустить LAPIC timer с периодом `hz` Гц на векторе 48.
///
/// Используется как fallback, когда PIC IRQ0 не доставляется (на части
/// ноутбуков 8259 не подключён, и таймер виден только через IOAPIC).
/// Калибровка идёт по монотонным часам ядра, а не по PIT channel 2:
/// PIT-канал 2 на некоторых чипсетах занят SMM/EC, и калибровка «плывёт».
///
/// EOI для этого вектора шлёт `isr_timer_apic` в idt.rs. `APIC_ACTIVE`
/// остаётся false: IRQ1 клавиатуры по-прежнему приходит через PIC и требует
/// PIC EOI.
pub fn start_periodic_timer(hz: u32) -> bool {
    if hz == 0 || !has_apic() {
        return false;
    }
    if let Some(base) = crate::acpi::get().as_ref().and_then(|a| a.lapic_addr) {
        if base != 0 {
            LAPIC_BASE_PHYS.store(base as u64, Ordering::SeqCst);
        }
    }

    unsafe {
        init_lapic();

        // Если MMIO не отвечает, все регистры читаются как 0xFFFFFFFF.
        if lapic_read(LAPIC_SVR) == 0xFFFF_FFFF {
            println!("[APIC] LAPIC MMIO not responding; fallback unavailable");
            return false;
        }

        // Калибровка: one-shot с максимальным значением, окно 10 мс.
        lapic_write(LAPIC_TIMER_DIV, 0x03); // divide by 16
        lapic_write(LAPIC_LVT_TIMER, 1 << 16); // masked, one-shot
        lapic_write(LAPIC_TIMER_INIT, 0xFFFF_FFFF);

        let t0 = crate::timer::uptime_us();
        while crate::timer::uptime_us().wrapping_sub(t0) < 10_000 {
            core::hint::spin_loop();
        }

        let remaining = lapic_read(LAPIC_TIMER_CURRENT);
        let elapsed = 0xFFFF_FFFFu32.wrapping_sub(remaining);
        if elapsed < 1_000 {
            println!("[APIC] LAPIC timer calibration failed (elapsed={})", elapsed);
            return false;
        }

        // elapsed — тиков за 10 мс; для периода 1/hz секунды:
        //   init = elapsed * 100 / hz
        let init = ((elapsed as u64 * 100) / hz as u64).clamp(1, u32::MAX as u64) as u32;
        lapic_write(LAPIC_TIMER_DIV, 0x03);
        lapic_write(LAPIC_LVT_TIMER, VEC_LAPIC_TIMER as u32 | (1 << 12)); // periodic
        lapic_write(LAPIC_TIMER_INIT, init);
        println!(
            "[APIC] LAPIC timer: {} Hz, vector {}, init={} (calibrated by {} clock)",
            hz,
            VEC_LAPIC_TIMER,
            init,
            crate::timer::current_source().name()
        );
    }

    true
}

pub const fn timer_vector() -> u8 {
    VEC_LAPIC_TIMER
}
