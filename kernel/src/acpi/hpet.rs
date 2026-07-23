// HPET — High Precision Event Timer
//
// Содержит базовый адрес HPET-регистров и количество компариторов.

use crate::acpi::sdt::SdtHeader;

/// Основная структура HPET таблицы (сразу после SDT-заголовка).
#[repr(C, packed)]
struct HpetTable {
    hardware_revision_id: u8,
    comparator_count: u8,
    _oem_attributes: u8,
    _pci_vendor_id: u16,
    address: HpetAddress,
    sequence_number: u8,
    _minimum_tick: u16,
    _page_protection: u8,
}

#[repr(C, packed)]
struct HpetAddress {
    address_space_id: u8,
    _register_bit_width: u8,
    _register_bit_offset: u8,
    _reserved: u8,
    address: u64,
}

/// Парсит HPET таблицу. Возвращает (base_address, comparator_count).
pub fn parse(hpet_phys: usize) -> Option<(u64, u8)> {
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let hpet_virt = hhdm as usize + hpet_phys;
    let header = unsafe { &*(hpet_virt as *const SdtHeader) };

    if !header.validate() {
        crate::println!("[ACPI] HPET: неверная контрольная сумма");
        return None;
    }

    if &header.signature != b"HPET" {
        crate::println!("[ACPI] HPET: неверная сигнатура");
        return None;
    }

    let table = unsafe { &*((hpet_virt + core::mem::size_of::<SdtHeader>()) as *const HpetTable) };

    // address_space_id == 0 означает Memory-mapped (System Memory)
    Some((table.address.address, table.comparator_count))
}
