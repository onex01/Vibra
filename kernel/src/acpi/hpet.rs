// HPET — High Precision Event Timer
//
// Содержит базовый адрес HPET-регистров и количество компариторов.

/// Парсит HPET таблицу. Возвращает (base_address, comparator_count).
/// Использует byte-level чтение для packed struct (без unaligned u64).
pub fn parse(hpet_phys: usize) -> Option<(u64, u8)> {
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let hpet_virt = hhdm as usize + hpet_phys;

    // Читаем заголовок SDT (36 байт) через copy — safe
    let mut sdt_hdr = [0u8; 36];
    unsafe {
        core::ptr::copy_nonoverlapping(hpet_virt as *const u8, sdt_hdr.as_mut_ptr(), 36);
    }
    let sig = [sdt_hdr[0], sdt_hdr[1], sdt_hdr[2], sdt_hdr[3]];

    if sig != *b"HPET" {
        return None;
    }

    // Тело HPET таблицы: сразу после SDT заголовка (36 байт)
    // Компактная раскладка: revision(1) + comp_count(1) + oem_attr(1) + pci_vendor(2) + HpetAddress(12) + seq(1) + min_tick(2) + prot(1)
    let body = hpet_virt + 36;

    // comparator_count — offset 1 от начала тела
    let comparator_count = unsafe { core::ptr::read_volatile((body + 1) as *const u8) };

    // HpetAddress начинается на offset 4: space_id(1) + bit_width(1) + bit_offset(1) + reserved(1) + address(8)
    // address — u64 на offset 4+4=8 от начала body. Может быть unaligned!
    let mut addr_buf = [0u8; 8];
    unsafe {
        core::ptr::copy_nonoverlapping((body + 8) as *const u8, addr_buf.as_mut_ptr(), 8);
    }
    let address = u64::from_le_bytes(addr_buf);

    crate::println!("[ACPI] HPET: base={:#x}, comparators={}", address, comparator_count);

    Some((address, comparator_count))
}
