// RSDP — Root System Description Pointer
//
// Ищем RSDP через Limine RsdpRequest, проверяем сигнатуру 'RSD PTR'
// и контрольную сумму.

use core::slice;

#[repr(C, packed)]
pub struct Rsdp {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    // Версия 2.0+ (revision >= 2)
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub _reserved: [u8; 3],
}

impl Rsdp {
    /// Проверяет сигнатуру и базовую контрольную сумму.
    pub fn validate(&self) -> bool {
        if &self.signature != b"RSD PTR " {
            return false;
        }
        let raw = unsafe {
            slice::from_raw_parts(self as *const Self as *const u8, 20)
        };
        let sum: u8 = raw.iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
        sum == 0
    }

    /// Проверяет расширенную контрольную сумму (для RS 2.0+).
    pub fn validate_extended(&self) -> bool {
        if self.revision < 2 {
            return true;
        }
        let raw = unsafe {
            slice::from_raw_parts(self as *const Self as *const u8, self.length as usize)
        };
        let sum: u8 = raw.iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
        sum == 0
    }
}

/// Получает RSDP через Limine RSDP Request.
/// Limine rev0: address = virtual (уже через HHDM)
/// Limine rev3+: address = physical (нужен HHDM offset)
/// Возвращает virtual address RSDP или None.
pub fn find_rsdp() -> Option<usize> {
    let response = crate::RSDP_REQUEST.response()?;
    let addr = response.address as usize;
    if addr == 0 {
        return None;
    }
    // Limine base revision determines if address is virtual or physical.
    // For base rev 0 (default): address is already virtual.
    // For base rev 3: address is physical, need HHDM offset.
    // Safe approach: try reading directly first, then with HHDM.
    let rsdp = unsafe { &*(addr as *const Rsdp) };
    if rsdp.validate() && rsdp.validate_extended() {
        return Some(addr); // already virtual or physical that happens to work
    }
    // Try adding HHDM (physical address mode)
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let virt = hhdm as usize + addr;
    let rsdp = unsafe { &*(virt as *const Rsdp) };
    if !rsdp.validate() {
        crate::println!("[ACPI] RSDP: неверная сигнатура или контрольная сумма");
        return None;
    }
    if !rsdp.validate_extended() {
        crate::println!("[ACPI] RSDP: неверная расширенная контрольная сумма");
        return None;
    }
    Some(addr)
}
