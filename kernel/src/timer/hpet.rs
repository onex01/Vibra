// HPET — High Precision Event Timer.
//
// HPET — MMIO-таймер x86 (обычно 0xFED00000), который даёт монотонный
// 64-битный счётчик. Он не зависит от доставки прерываний, поэтому именно
// он (или TSC, откалиброванный по нему) используется как источник времени.
//
// Регистры:
//   0x000 ID       — [63:32] период счётчика в фемтосекундах,
//                    [15:0] vendor ID, [13] поддержка 64-битного счётчика
//   0x010 CFG      — [0] ENABLE_CNF, [1] LEG_RT_CNF (legacy replacement)
//   0x020 ISR      — статус прерываний (read-only)
//   0x0F0 COUNTER  — 64-битный основной счётчик (lo 0x0F0, hi 0x0F4)
//
// ВАЖНО: частота HPET НЕ определяется по vendor ID. Спецификация требует
// брать её из поля PERIOD регистра ID: counter_hz = 1e15 / period_fs.
// Раньше здесь частота угадывалась по PCI vendor (24 МГц для Intel,
// 14.318 МГц для AMD), из-за чего `sleep_ms` и uptime на реальном железе
// расходились в разы — причём на разных машинах по-разному.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Стандартный физический адрес HPET (используется, если ACPI не дал свой).
const HPET_DEFAULT_PHYS: u64 = 0xFED0_0000;

const REG_ID: u64 = 0x000;
const REG_CFG: u64 = 0x010;
const REG_COUNTER_LO: u64 = 0x0F0;
const REG_COUNTER_HI: u64 = 0x0F4;

const CFG_ENABLE: u64 = 1 << 0;
const CFG_LEGACY_ROUTE: u64 = 1 << 1;
const ID_COUNTER_64BIT: u64 = 1 << 13;

/// Разумные границы частоты HPET: 1 кГц .. 100 МГц.
const MIN_HZ: u64 = 1_000;
const MAX_HZ: u64 = 100_000_000;

static HPET_BASE: AtomicU64 = AtomicU64::new(0);
static HPET_PERIOD_FS: AtomicU64 = AtomicU64::new(0);
static HPET_HZ: AtomicU64 = AtomicU64::new(0);
static HPET_READY: AtomicBool = AtomicBool::new(false);

#[inline]
fn hhdm_virt(phys: u64) -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed) + phys
}

unsafe fn read32(virt: u64, offset: u64) -> u32 {
    core::ptr::read_volatile((virt + offset) as *const u32)
}

unsafe fn read64(virt: u64, offset: u64) -> u64 {
    let lo = read32(virt, offset) as u64;
    let hi = read32(virt, offset + 4) as u64;
    (hi << 32) | lo
}

unsafe fn write64(virt: u64, offset: u64, val: u64) {
    core::ptr::write_volatile((virt + offset) as *mut u32, val as u32);
    core::ptr::write_volatile((virt + offset + 4) as *mut u32, (val >> 32) as u32);
}

/// Проверить один кандидат-адрес HPET: период, 64-битность, «тикает ли».
unsafe fn probe(phys: u64) -> Option<(u64, u64)> {
    let virt = hhdm_virt(phys);

    let id = read64(virt, REG_ID);
    let period_fs = id >> 32;
    if period_fs == 0 {
        return None;
    }
    let hz = 1_000_000_000_000_000u64 / period_fs;
    if !(MIN_HZ..=MAX_HZ).contains(&hz) {
        return None;
    }
    if id & ID_COUNTER_64BIT == 0 {
        // 32-битный счётчик переполняется каждые ~7 минут при 10 МГц —
        // для системных часов это неприемлемо. Пусть решает TSC/PIT.
        crate::println!(
            "[HPET] {:#x}: 32-bit counter only (period {} fs); skipping",
            phys,
            period_fs
        );
        return None;
    }

    // Legacy replacement выключаем: иначе HPET подменяет IRQ0/IRQ2 PIT,
    // и запрограммированный нами PIT-тик перестаёт соответствовать 100 Гц.
    let cfg = read64(virt, REG_CFG);
    let cfg = cfg & !CFG_LEGACY_ROUTE;
    write64(virt, REG_CFG, cfg);
    write64(virt, REG_CFG, cfg | CFG_ENABLE);

    // Проверяем, что счётчик действительно идёт.
    let c0 = read64(virt, REG_COUNTER_LO);
    let mut advanced = false;
    for _ in 0..100_000u32 {
        if read64(virt, REG_COUNTER_LO) != c0 {
            advanced = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !advanced {
        crate::println!("[HPET] {:#x}: counter does not advance", phys);
        return None;
    }

    Some((period_fs, hz))
}

/// Инициализация HPET. `acpi_base` — адрес из таблицы ACPI HPET (если есть).
pub fn init(acpi_base: Option<u64>) -> bool {
    let mut candidates = [0u64; 2];
    let mut count = 0usize;
    if let Some(base) = acpi_base.filter(|b| *b != 0) {
        candidates[count] = base;
        count += 1;
    }
    if acpi_base != Some(HPET_DEFAULT_PHYS) {
        candidates[count] = HPET_DEFAULT_PHYS;
        count += 1;
    }

    for &phys in &candidates[..count] {
        if let Some((period_fs, hz)) = unsafe { probe(phys) } {
            HPET_BASE.store(phys, Ordering::SeqCst);
            HPET_PERIOD_FS.store(period_fs, Ordering::SeqCst);
            HPET_HZ.store(hz, Ordering::SeqCst);
            HPET_READY.store(true, Ordering::SeqCst);
            crate::println!(
                "[HPET] base={:#x} period={} fs ({} Hz), 64-bit counter enabled",
                phys,
                period_fs,
                hz
            );
            return true;
        }
    }

    crate::println!("[HPET] No usable HPET found");
    false
}

/// Прочитать 64-битный основной счётчик.
///
/// Чтение согласовано: hi читается дважды, чтобы не поймать перенос между
/// чтениями lo/hi.
pub fn read_counter() -> u64 {
    if !HPET_READY.load(Ordering::Relaxed) {
        return 0;
    }
    let virt = hhdm_virt(HPET_BASE.load(Ordering::Relaxed));
    unsafe {
        loop {
            let hi1 = read32(virt, REG_COUNTER_HI) as u64;
            let lo = read32(virt, REG_COUNTER_LO) as u64;
            let hi2 = read32(virt, REG_COUNTER_HI) as u64;
            if hi1 == hi2 {
                return (hi2 << 32) | lo;
            }
        }
    }
}

/// Частота счётчика HPET в герцах.
pub fn counter_hz() -> u64 {
    HPET_HZ.load(Ordering::Relaxed)
}

/// Период счётчика HPET в фемтосекундах.
pub fn period_fs() -> u64 {
    HPET_PERIOD_FS.load(Ordering::Relaxed)
}

/// Физический базовый адрес HPET.
pub fn physical_address() -> u64 {
    HPET_BASE.load(Ordering::Relaxed)
}

pub fn is_ready() -> bool {
    HPET_READY.load(Ordering::Relaxed)
}
