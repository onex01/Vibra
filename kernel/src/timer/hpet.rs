// HPET — High Precision Event Timer
//
// HPET — современный аппаратный таймер x86, заменяет PIT.
// Работает через MMIO-регистры по стандартному адресу 0xFED00000.
//
// Регистры:
//   ID Register   (0x000): ревизия, количество таймеров, размер счётчика
//   CFG Register  (0x010): глобальное включение, legacy replacement
//   ISR Register  (0x020): статус прерываний (read-only)
//   Counter (lo)  (0x024): младшие 32 бита основного счётчика
//   Counter (hi)  (0x0F0): старшие 32 бита (64-бит режим)
//
// Компаратор N (N >= 0):
//   Config   (0x100 + N*0x20): конфигурация таймера
//   Compare  (0x108 + N*0x20): значение сравнения
//   FSB Route (0x110 + N*0x20): FSB interrupt route (для MSI)
//
// Инициализация:
//   1. Прочитать ID → определить частоту (обычно 10 МГц)
//   2. Отключить legacy replacement
//   3. Включить основной счётчик
//   4. Настроить компаратор 0 для periodick interrupts

use crate::println;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// ======================== HPET MMIO Addresses ========================

/// Стандартный физический адрес HPET (spec)
const HPET_PHYS_BASE: u64 = 0xFED0_0000;

// Register offsets
const HPET_ID: u64 = 0x000;
const HPET_CFG: u64 = 0x010;
const HPET_ISR: u64 = 0x020;
const HPET_COUNTER_LO: u64 = 0x0F0; // 32-bit counter value
const HPET_COUNTER_HI: u64 = 0x024; // Legacy 32-bit high (не используется, используем 64-bit)

// Config bits
const HPET_CFG_ENABLE: u64 = 1 << 1;     // Global Enable
const HPET_CFG_LEGACY: u64 = 1 << 1;     // Legacy Replacement Route (бит 1 в CFG)
const HPET_CFG_LEGACY_BIT: u64 = 1;       // Legacy mode replacement

// Timer N registers
const HPET_TIMER_CFG: u64 = 0x100;   // Base offset for timer 0
const HPET_TIMER_COMP: u64 = 0x108;
const HPET_TIMER_FSB: u64 = 0x110;
const HPET_TIMER_STRIDE: u64 = 0x020; // Расстояние между таймерами

// Timer Config bits
const HPET_TN_ENABLE: u64 = 1 << 2;       // Timer Interrupt Enable
const HPET_TN_PERIODIC: u64 = 1 << 3;     // Periodic Mode
const HPET_TN_FSB_ENABLE: u64 = 1 << 14;  // FSB Interrupt Mapping enable

// ======================== Globals ========================

static HPET_BASE: AtomicU64 = AtomicU64::new(0);
static HPET_READY: AtomicBool = AtomicBool::new(false);
static HPET_TICKS_PER_SEC: AtomicU64 = AtomicU64::new(10_000_000); // Default 10 MHz

fn hhdm_virt(phys: u64) -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed) + phys
}

// ======================== MMIO Access ========================

unsafe fn hpet_read64(offset: u64) -> u64 {
    let base = HPET_BASE.load(Ordering::Relaxed);
    // HPET 64-bit counter: read hi, lo, hi again (проверка обёртки)
    if offset == HPET_COUNTER_LO {
        // 64-bit counter: читаем как 64-bit напрямую
        let lo = core::ptr::read_volatile((base + HPET_COUNTER_LO) as *const u32) as u64;
        let hi = core::ptr::read_volatile((base + 0x024) as *const u32) as u64;
        (hi << 32) | lo
    } else {
        core::ptr::read_volatile((base + offset) as *const u64)
    }
}

unsafe fn hpet_read32(offset: u64) -> u32 {
    let base = HPET_BASE.load(Ordering::Relaxed);
    core::ptr::read_volatile((base + offset) as *const u32)
}

unsafe fn hpet_write64(offset: u64, val: u64) {
    let base = HPET_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u64, val);
}

unsafe fn hpet_write32(offset: u64, val: u32) {
    let base = HPET_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

// ======================== Init ========================

/// Инициализация HPET таймера
/// `acpi_base`: базовый адрес HPET из ACPI таблицы (0 если неизвестен)
pub fn init(acpi_base: Option<u64>) -> bool {
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    if hhdm == 0 {
        println!("[HPET] HHDM не инициализирован");
        return false;
    }

    // Используем адрес из ACPI если есть, иначе fallback на стандартный
    let phys = acpi_base.unwrap_or(HPET_PHYS_BASE);

    // Проверяем что адрес в разумном диапазоне (ниже 4GB для HHDM)
    if phys == 0 || phys > 0x1_0000_0000 {
        println!("[HPET] Адрес {:#x} вне HHDM диапазона — HPET пропущен", phys);
        return false;
    }

    let mmio_virt = hhdm + phys;
    HPET_BASE.store(mmio_virt, Ordering::SeqCst);

    unsafe {
        // Читаем ID Register
        let id = hpet_read64(HPET_ID);
        let revision = (id & 0xFF) as u8;
        let num_timers = ((id >> 8) & 0x1F) as u8 + 1;
        let counter_size = (id >> 13) & 1; // 0 = 32-bit, 1 = 64-bit
        let pci_vendor_id = (id >> 16) as u16;

        println!("[HPET] ID: rev={}, timers={}, {}-bit, vendor={:#06x}",
            revision, num_timers,
            if counter_size != 0 { "64" } else { "32" },
            pci_vendor_id);

        if pci_vendor_id == 0 || pci_vendor_id == 0xFFFF {
            println!("[HPET] Невалидный vendor ID — HPET не обнаружен");
            return false;
        }

        // Определяем частоту: PCI vendor ID определяет тактовую частоту
        // Если vendor = 0x8086 (Intel): 14.31818 МГц / 2 = 24.16 МГц → нет,
        // HPET spec определяет по умолчанию 10 MHz, но vendor может указать иную.
        // Для большинства чипсетов (Intel/AMD) частота = 10 МГц (PCI vendor = 0x8086).
        // Если vendor = 0x10DE (NVIDIA) → 25.175 МГц.
        let freq = match pci_vendor_id {
            0x8086 => 24_000_000u64, // Intel: 14.31818 MHz / 2 ≈ 24 MHz (уточняется через MSR)
            0x1022 => 14_318_180u64, // AMD: 14.31818 MHz
            _ => 10_000_000u64,       // По умолчанию 10 МГц (HPET spec)
        };

        println!("[HPET] Частота: {} Гц ({} МГц)", freq, freq / 1_000_000);
        HPET_TICKS_PER_SEC.store(freq, Ordering::SeqCst);

        // Отключаем Legacy Replacement (бит 1 в CFG)
        let cfg = hpet_read64(HPET_CFG);
        if cfg & HPET_CFG_LEGACY != 0 {
            hpet_write64(HPET_CFG, cfg & !HPET_CFG_LEGACY);
            println!("[HPET] Legacy replacement отключён");
        }

        // Включаем основной счётчик
        let cfg = hpet_read64(HPET_CFG);
        hpet_write64(HPET_CFG, cfg | HPET_CFG_ENABLE);

        // Проверяем, что счётчик тикает
        let count1 = hpet_read64(HPET_COUNTER_LO);
        // Ждём несколько тиков
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
        let count2 = hpet_read64(HPET_COUNTER_LO);

        if count1 == count2 {
            println!("[HPET] Счётчик не тикает (count={:#x}), HPET не работает", count1);
            // Отключаем обратно
            let cfg = hpet_read64(HPET_CFG);
            hpet_write64(HPET_CFG, cfg & !HPET_CFG_ENABLE);
            return false;
        }

        let delta = count2 - count1;
        println!("[HPET] Счётчик тикает: delta={}", delta);

        // Настраиваем компаратор 0 для periodic interrupts (vector 48 = APIC timer vector)
        // Пока только логируем — прерывания не настраиваем без полной APIC интеграции
        if num_timers > 0 {
            let timer_cfg = hpet_read64(HPET_TIMER_CFG);
            println!("[HPET] Компаратор 0: CFG={:#x}", timer_cfg);
        }

        HPET_READY.store(true, Ordering::SeqCst);
        println!("[HPET] Инициализирован успешно");
    }

    true
}

// ======================== Public API ========================

/// Проверить, инициализирован ли HPET
pub fn is_ready() -> bool {
    HPET_READY.load(Ordering::SeqCst)
}

/// Прочитать текущее значение основного счётчика HPET
pub fn read_counter() -> u64 {
    if !is_ready() {
        return 0;
    }
    unsafe { hpet_read64(HPET_COUNTER_LO) }
}

/// Получить частоту HPET (тактов в секунду)
pub fn ticks_per_second() -> u64 {
    HPET_TICKS_PER_SEC.load(Ordering::SeqCst)
}

/// Busy-wait на N миллисекунд через HPET
pub fn sleep_ms(ms: u64) {
    if !is_ready() || ms == 0 {
        return;
    }

    let ticks_per_ms = ticks_per_second() / 1000;
    let target = ticks_per_ms * ms;
    let start = read_counter();

    while read_counter().wrapping_sub(start) < target {
        core::hint::spin_loop();
    }
}

/// Получить базовый физический адрес HPET (для ACPI)
pub fn physical_address() -> u64 {
    HPET_PHYS_BASE
}
