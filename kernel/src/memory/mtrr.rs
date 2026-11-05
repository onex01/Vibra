// memory/mtrr.rs — MTRR: типы памяти для физических диапазонов.
//
// Зачем это здесь. Фреймбуфер на реальном железе часто помечен в MTRR как
// UC (uncached). PAT сам по себе это не переопределяет: эффективный тип —
// пересечение PAT и MTRR, и если MTRR говорит UC, любая запись пикселя
// уходит отдельной транзакцией по шине. Именно поэтому на ноутбуке
// прокрутка стоила ~40 мс на строку, хотя `PAT WC` был запрограммирован.
//
// Модуль умеет:
//   * читать MTRR (сколько диапазонов, default type, какие включены);
//   * определять тип, покрывающий физический адрес;
//   * помечать диапазон как write-combining (WC) — только если есть
//     свободный слот или уже есть диапазон ровно на этот диапазон.
//
// Изменение MTRR требует сброса кэшей (`wbinvd`), иначе уже закэшированные
// строки останутся с прежним типом и память станет несогласованной.

use core::sync::atomic::{AtomicUsize, Ordering};

const IA32_MTRRCAP: u32 = 0xFE;
const IA32_MTRR_DEF_TYPE: u32 = 0x2FF;
const IA32_MTRR_PHYSBASE0: u32 = 0x200;
const IA32_MTRR_PHYSMASK0: u32 = 0x201;

const MTRR_TYPE_UC: u8 = 0;
const MTRR_TYPE_WC: u8 = 1;
const MTRR_TYPE_WT: u8 = 4;
const MTRR_TYPE_WP: u8 = 5;
const MTRR_TYPE_WB: u8 = 6;

const MASK_VALID: u64 = 1 << 11;
const DEF_TYPE_ENABLE: u64 = 1 << 11;

/// Сколько диапазонов удалось перевести в WC (для `diag fb`).
static WC_RANGES: AtomicUsize = AtomicUsize::new(0);
/// Тип, покрывавший фреймбуфер до нашей правки (0xFF — не покрыт).
static FB_TYPE_BEFORE: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Тип, покрывающий фреймбуфер после правки.
static FB_TYPE_AFTER: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nostack, preserves_flags)
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

/// Сколько переменных диапазонов поддерживает CPU (MTRRCAP.VCNT).
pub fn variable_count() -> usize {
    let raw = unsafe { (rdmsr(IA32_MTRRCAP) & 0xFF) as usize };
    // Спецификация допускает до 255, но реально бывает 8–16. Если MSR
    // прочитался как 0xFF (сбой/эмуляция), обход уйдёт на несуществующие
    // MSR и получит #GP — ограничиваем.
    raw.min(16)
}

/// Поддерживает ли CPU тип WC (MTRRCAP bit 10).
pub fn supports_wc() -> bool {
    unsafe { rdmsr(IA32_MTRRCAP) & (1 << 10) != 0 }
}

/// Тип памяти по умолчанию (IA32_MTRR_DEF_TYPE bits 7:0).
pub fn default_type() -> u8 {
    unsafe { (rdmsr(IA32_MTRR_DEF_TYPE) & 0xFF) as u8 }
}

pub fn type_name(t: u8) -> &'static str {
    match t {
        MTRR_TYPE_UC => "UC",
        MTRR_TYPE_WC => "WC",
        2 => "UC-",
        3 => "UC-",
        MTRR_TYPE_WT => "WT",
        MTRR_TYPE_WP => "WP",
        MTRR_TYPE_WB => "WB",
        _ => "?",
    }
}

/// Тип, покрывающий физический адрес. `None` — не покрыт ни одним
/// диапазоном, действует `default_type()`.
pub fn covering_type(physical: u64) -> Option<u8> {
    let count = variable_count();
    for index in 0..count {
        unsafe {
            let base = rdmsr(IA32_MTRR_PHYSBASE0 + (index as u32) * 2);
            let mask = rdmsr(IA32_MTRR_PHYSMASK0 + (index as u32) * 2);
            if mask & MASK_VALID == 0 {
                continue;
            }
            let range_mask = mask & 0x000F_FFFF_FFFF_F000;
            if physical & range_mask == base & range_mask {
                return Some((base & 0xFF) as u8);
            }
        }
    }
    None
}

/// Список активных диапазонов: `(base, size, type)`.
pub fn ranges() -> alloc::vec::Vec<(u64, u64, u8)> {
    let mut out = alloc::vec::Vec::new();
    for index in 0..variable_count() {
        unsafe {
            let base = rdmsr(IA32_MTRR_PHYSBASE0 + (index as u32) * 2);
            let mask = rdmsr(IA32_MTRR_PHYSMASK0 + (index as u32) * 2);
            if mask & MASK_VALID == 0 {
                continue;
            }
            let range_mask = mask & 0x000F_FFFF_FFFF_F000;
            // Маска — это последовательность единиц от бита N до 51, поэтому
            // размер = 1 << N, где N — младший установленный бит. Простой
            // `!mask + 1` давал мусор: биты выше 51 тоже инвертировались.
            let size = if range_mask == 0 {
                0
            } else {
                1u64 << range_mask.trailing_zeros()
            };
            out.push((base & 0x000F_FFFF_FFFF_F000, size, (base & 0xFF) as u8));
        }
    }
    out
}

/// Диапазон, выровненный под требования MTRR: размер — степень двойки,
/// база выровнена по размеру.
fn align_range(physical: u64, length: u64) -> Option<(u64, u64)> {
    if length == 0 {
        return None;
    }
    let size = length.next_power_of_two();
    if size > (1u64 << 40) {
        return None;
    }
    let base = physical & !(size - 1);
    Some((base, size))
}

/// Пометить диапазон как write-combining.
///
/// Возвращает `true`, если тип гарантированно WC после вызова. Если диапазон
/// уже покрыт другим MTRR с иным размером, ничего не меняем: переписывать
/// чужой диапазон опаснее, чем оставить фреймбуфер медленным.
pub fn set_write_combining(physical: u64, length: u64) -> bool {
    if !supports_wc() {
        return false;
    }
    let Some((base, size)) = align_range(physical, length) else {
        return false;
    };

    let before = covering_type(physical).unwrap_or(default_type());
    FB_TYPE_BEFORE.store(before as usize, Ordering::Relaxed);
    if before == MTRR_TYPE_WC {
        FB_TYPE_AFTER.store(MTRR_TYPE_WC as usize, Ordering::Relaxed);
        return true;
    }

    let count = variable_count();
    let mask_bits = !(size - 1) & 0x000F_FFFF_FFFF_F000;

    unsafe {
        // Ищем диапазон ровно на этот размер: тогда можно сменить только тип.
        let mut target: Option<usize> = None;
        let mut free: Option<usize> = None;
        for index in 0..count {
            let base_msr = rdmsr(IA32_MTRR_PHYSBASE0 + (index as u32) * 2);
            let mask_msr = rdmsr(IA32_MTRR_PHYSMASK0 + (index as u32) * 2);
            if mask_msr & MASK_VALID == 0 {
                if free.is_none() {
                    free = Some(index);
                }
                continue;
            }
            let range_mask = mask_msr & 0x000F_FFFF_FFFF_F000;
            if range_mask == mask_bits && (base_msr & 0x000F_FFFF_FFFF_F000) == base {
                target = Some(index);
                break;
            }
        }

        let slot = match target.or(free) {
            Some(slot) => slot,
            // Все диапазоны заняты другими областями: не трогаем их.
            None => {
                FB_TYPE_AFTER.store(before as usize, Ordering::Relaxed);
                return false;
            }
        };

        // MTRR должны быть включены, иначе запись в них бессмысленна.
        let def = rdmsr(IA32_MTRR_DEF_TYPE);
        if def & DEF_TYPE_ENABLE == 0 {
            wrmsr(IA32_MTRR_DEF_TYPE, def | DEF_TYPE_ENABLE);
        }

        // Перед изменением типа памяти кэш обязан быть сброшен: иначе в нём
        // останутся строки с прежним типом.
        core::arch::asm!("wbinvd", options(nostack));

        wrmsr(
            IA32_MTRR_PHYSBASE0 + (slot as u32) * 2,
            base | MTRR_TYPE_WC as u64,
        );
        wrmsr(
            IA32_MTRR_PHYSMASK0 + (slot as u32) * 2,
            mask_bits | MASK_VALID,
        );

        core::arch::asm!("wbinvd", options(nostack));
    }

    let after = covering_type(physical).unwrap_or(default_type());
    FB_TYPE_AFTER.store(after as usize, Ordering::Relaxed);
    if after == MTRR_TYPE_WC {
        WC_RANGES.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// `(диапазонов переведено, тип до, тип после)`; `usize::MAX` — не покрыт.
pub fn framebuffer_state() -> (usize, usize, usize) {
    (
        WC_RANGES.load(Ordering::Relaxed),
        FB_TYPE_BEFORE.load(Ordering::Relaxed),
        FB_TYPE_AFTER.load(Ordering::Relaxed),
    )
}

/// Отчёт для `diag fb`: все активные диапазоны и тип по умолчанию.
pub fn report() -> alloc::string::String {
    use alloc::format;
    let mut text = format!(
        "  mtrr: {} range(s), default {}, WC {}\n",
        variable_count(),
        type_name(default_type()),
        if supports_wc() { "supported" } else { "unsupported" }
    );
    for (base, size, kind) in ranges() {
        text.push_str(&format!(
            "    {:#012x}-{:#012x} ({} MiB) {}\n",
            base,
            base + size,
            size / (1024 * 1024),
            type_name(kind)
        ));
    }
    text
}
