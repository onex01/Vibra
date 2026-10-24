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
    //   36: Hardware Rev ID (u8)
    //   37: Comparator Count (bits 0-4) | Counter Size 64-bit (bit 5)
    //   38: PCI Vendor ID (u16)
    //   40: Address Space ID / bit width / bit offset / reserved
    //   44: Address (u64, generic address structure)
    //   52: HPET sequence number, 53: minimum tick, 55: page protection
    let body = hpet_virt + 36;

    let flags = unsafe { core::ptr::read_volatile((body + 1) as *const u8) };
    let comparator_count = flags & 0x1F; // bits 0-4, не весь байт
    let counter_64bit = flags & 0x20 != 0;

    // HpetAddress начинается на offset 8 от начала body (36 + 8 = 44).
    let mut addr_buf = [0u8; 8];
    unsafe {
        core::ptr::copy_nonoverlapping((body + 8) as *const u8, addr_buf.as_mut_ptr(), 8);
    }
    let address = u64::from_le_bytes(addr_buf);

    crate::println!(
        "[ACPI] HPET: base={:#x}, comparators={}, counter={}-bit",
        address,
        comparator_count,
        if counter_64bit { 64 } else { 32 }
    );

    Some((address, comparator_count))
}
