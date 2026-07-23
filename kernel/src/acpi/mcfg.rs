// MCFG — PCI Express Memory-Mapped Configuration
//
// Содержит базовый адрес MMCONFIG для ECAM (Enhanced Configuration Access
// Mechanism), что позволяет обращаться к конфигурационному пространству
// PCIe через MMIO.

use crate::acpi::sdt::SdtHeader;

/// Структура MCFG-записи (28 байт для одной шины).
#[repr(C, packed)]
struct McfgEntry {
    base_address: u64,
    segment_group: u16,
    start_bus: u8,
    end_bus: u8,
    _reserved: u32,
}

/// Парсит MCFG таблицу. Возвращает MMCONFIG base address.
pub fn parse(mcfg_phys: usize) -> Option<u64> {
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let mcfg_virt = hhdm as usize + mcfg_phys;
    let header = unsafe { &*(mcfg_virt as *const SdtHeader) };

    if !header.validate() {
        crate::println!("[ACPI] MCFG: неверная контрольная сумма");
        return None;
    }

    if &header.signature != b"MCFG" {
        crate::println!("[ACPI] MCFG: неверная сигнатура");
        return None;
    }

    let header_size = core::mem::size_of::<SdtHeader>();
    let entries_bytes = (header.length as usize).saturating_sub(header_size + 4); // +4 для reserved
    let entry_count = entries_bytes / core::mem::size_of::<McfgEntry>();

    if entry_count == 0 {
        return None;
    }

    // Берём базу из первой записи (обычно сегмент 0, шины 0-255)
    let entries_base = mcfg_virt + header_size + 4; // +4 для reserved после заголовка
    let first_entry = unsafe { &*(entries_base as *const McfgEntry) };
    Some(first_entry.base_address)
}
