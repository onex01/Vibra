// SDT — System Description Table
//
// Заголовок SDT общий для RSDT/XSDT и всех подтаблиц.
// XSDT содержит 64-битные адреса подтаблиц.

/// Заголовок любой SDT-таблицы (36 байт).
#[repr(C, packed)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

impl SdtHeader {
    /// Проверяет контрольную сумму таблицы по всему объёму `length`.
    pub fn validate(&self) -> bool {
        let raw = unsafe {
            core::slice::from_raw_parts(self as *const Self as *const u8, self.length as usize)
        };
        let sum: u8 = raw.iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
        sum == 0
    }
}

/// Прочитать unaligned u32
unsafe fn read_u32_unaligned(addr: usize) -> u32 {
    let mut buf = [0u8; 4];
    core::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), 4);
    u32::from_le_bytes(buf)
}

/// Прочитать unaligned u64
unsafe fn read_u64_unaligned(addr: usize) -> u64 {
    let mut buf = [0u8; 8];
    core::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), 8);
    u64::from_le_bytes(buf)
}

/// Перебирает все SDT-таблицы, указанные в XSDT/RSDT.
/// Возвращает вектор физических адресов таблиц.
pub fn enumerate_xsdt(xsdt_phys: u64) -> alloc::vec::Vec<usize> {
    let mut result = alloc::vec::Vec::new();
    if xsdt_phys == 0 {
        return result;
    }
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let xsdt_virt = (hhdm + xsdt_phys) as usize;

    // Читаем заголовок через copy_nonoverlapping (unaligned-safe)
    let header_size = core::mem::size_of::<SdtHeader>();
    let mut hdr = [0u8; 36];
    unsafe {
        core::ptr::copy_nonoverlapping(xsdt_virt as *const u8, hdr.as_mut_ptr(), header_size);
    }
    let sig = [hdr[0], hdr[1], hdr[2], hdr[3]];
    let xsdt_len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;

    let is_xsdt = sig == *b"XSDT";
    let is_rsdt = sig == *b"RSDT";
    if !is_xsdt && !is_rsdt {
        crate::println!("[ACPI] SDT: неверная сигнатура {:?}", core::str::from_utf8(&sig));
        return result;
    }
    if xsdt_len < header_size || xsdt_len > 0x10000 {
        crate::println!("[ACPI] SDT: некорректная длина {}", xsdt_len);
        return result;
    }

    let entry_size = if is_xsdt { 8 } else { 4 };
    let entries_bytes = xsdt_len - header_size;
    let entry_count = entries_bytes / entry_size;
    let tag = if is_xsdt { "XSDT" } else { "RSDT" };

    crate::println!("[ACPI] {}: {} подтаблиц (len={})", tag, entry_count, xsdt_len);

    let table_base = xsdt_virt + header_size;
    for i in 0..entry_count {
        let entry_addr = table_base + i * entry_size;
        if entry_addr + entry_size > xsdt_virt + xsdt_len {
            break;
        }
        let addr = if is_xsdt {
            unsafe { read_u64_unaligned(entry_addr) as usize }
        } else {
            unsafe { read_u32_unaligned(entry_addr) as usize }
        };
        if addr != 0 {
            result.push(addr);
        }
    }

    result
}
