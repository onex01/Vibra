// MADT — Multiple APIC Description Table

use alloc::vec::Vec;

/// Результат парсинга MADT.
pub struct MadtInfo {
    pub lapic_addr: u32,
    pub io_apic_addr: Option<u32>,
    pub io_apic_id: Option<u8>,
    pub iso: Vec<(u8, u32)>,
}

/// Читает unaligned u32 через copy (safe для невыровненных адресов)
unsafe fn read_u32(ptr: usize) -> u32 {
    let mut buf = [0u8; 4];
    core::ptr::copy_nonoverlapping(ptr as *const u8, buf.as_mut_ptr(), 4);
    u32::from_le_bytes(buf)
}

/// Читает unaligned u64 через copy
unsafe fn read_u64(ptr: usize) -> u64 {
    let mut buf = [0u8; 8];
    core::ptr::copy_nonoverlapping(ptr as *const u8, buf.as_mut_ptr(), 8);
    u64::from_le_bytes(buf)
}

pub fn parse(madt_phys: usize) -> Option<MadtInfo> {
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);
    let madt_virt = hhdm as usize + madt_phys;

    // Читаем заголовок SDT через copy (safe)
    let mut sdt_hdr = [0u8; 36];
    unsafe {
        core::ptr::copy_nonoverlapping(madt_virt as *const u8, sdt_hdr.as_mut_ptr(), 36);
    }
    let sig = [sdt_hdr[0], sdt_hdr[1], sdt_hdr[2], sdt_hdr[3]];
    let table_len = u32::from_le_bytes([sdt_hdr[4], sdt_hdr[5], sdt_hdr[6], sdt_hdr[7]]) as usize;

    if sig != *b"APIC" {
        crate::println!("[ACPI] MADT: неверная сигнатура {:?}", core::str::from_utf8(&sig));
        return None;
    }

    // MadtHeader immediately after SDT header (8 bytes)
    let madt_hdr_offset = madt_virt + 36;
    let lapic_addr = unsafe { read_u32(madt_hdr_offset) };
    let _flags = unsafe { read_u32(madt_hdr_offset + 4) };

    let mut info = MadtInfo {
        lapic_addr,
        io_apic_addr: None,
        io_apic_id: None,
        iso: Vec::new(),
    };

    // Records start after SDT header + MadtHeader
    let records_start = madt_virt + 36 + 8;
    let records_end = madt_virt + table_len;
    let mut ptr = records_start;

    while ptr + 2 <= records_end {
        let record_type = unsafe { core::ptr::read_volatile(ptr as *const u8) };
        let record_len = unsafe { core::ptr::read_volatile((ptr + 1) as *const u8) } as usize;
        if record_len < 2 {
            break;
        }

        match record_type {
            1 => {
                // IO APIC: id(u8@2), reserved(u8@3), addr(u32@4), gsi_base(u32@8)
                let ioapic_id = unsafe { core::ptr::read_volatile((ptr + 2) as *const u8) };
                let ioapic_addr = unsafe { read_u32(ptr + 4) };
                info.io_apic_addr = Some(ioapic_addr);
                info.io_apic_id = Some(ioapic_id);
            }
            2 => {
                // Interrupt Source Override: bus(u8@2), source(u8@3), gsi(u32@4)
                let bus = unsafe { core::ptr::read_volatile((ptr + 2) as *const u8) };
                let source = unsafe { core::ptr::read_volatile((ptr + 3) as *const u8) };
                let gsi = unsafe { read_u32(ptr + 4) };
                if bus == 0 {
                    info.iso.push((source, gsi));
                }
            }
            5 => {
                // Local APIC Address Override: addr(u64@4)
                let lapic_64 = unsafe { read_u64(ptr + 4) };
                info.lapic_addr = lapic_64 as u32;
            }
            _ => {}
        }

        ptr += record_len;
    }

    crate::println!("[ACPI] MADT: LAPIC={:#x}, IOAPIC={:?}, ISO={}",
        info.lapic_addr, info.io_apic_addr, info.iso.len());

    Some(info)
}
