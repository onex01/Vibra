// ACPI — Advanced Configuration and Power Interface
//
// Модуль парсит ключевые ACPI-таблицы: MADT (APIC), MCFG (PCIe ECAM),
// HPET (таймер). RSDP и XSDT используются как точка входа.
//
// Инициализация вызывается после memory::init().

pub mod rsdp;
pub mod sdt;
pub mod madt;
pub mod mcfg;
pub mod hpet;

use alloc::vec::Vec;
use spin::Mutex;

pub struct AcpiInfo {
    pub lapic_addr: Option<u32>,
    pub io_apic_addr: Option<u32>,
    pub io_apic_id: Option<u8>,
    pub iso: Vec<madt::IsoEntry>,
    pub mcfg_base: Option<u64>,
    pub hpet_base: Option<u64>,
    pub hpet_comparator_count: Option<u8>,
    /// FADT IA-PC Boot Architecture Flags (offset 109). Bit 1 = 8042 present.
    pub boot_arch_flags: Option<u16>,
}

pub static ACPI_INFO: Mutex<Option<AcpiInfo>> = Mutex::new(None);

/// Получить текущий AcpiInfo (блокировка).
pub fn get() -> spin::MutexGuard<'static, Option<AcpiInfo>> {
    ACPI_INFO.lock()
}

/// Есть ли на машине i8042 (PS/2) контроллер по данным FADT.
///
/// Если ACPI/FADT недоступны — возвращаем `true`: лучше попробовать
/// проинициализировать контроллер, чем молча остаться без клавиатуры.
pub fn has_8042() -> bool {
    let info = ACPI_INFO.lock();
    match info.as_ref().and_then(|i| i.boot_arch_flags) {
        Some(flags) => flags & 0x0002 != 0,
        None => true,
    }
}

/// Инициализация ACPI: RSDP → XSDT → MADT/MCFG/HPET.
pub fn init() {
    crate::println!("[ACPI] Searching for RSDP...");

    let rsdp_virt = match rsdp::find_rsdp() {
        Some(v) => v,
        None => {
            crate::println!("[ACPI] RSDP not found; ACPI unavailable");
            return;
        }
    };

    crate::println!("[ACPI] RSDP found at {:#x}", rsdp_virt);

    // find_rsdp() возвращает виртуальный адрес, доступный напрямую
    let rsdp = unsafe { &*(rsdp_virt as *const rsdp::Rsdp) };

    // Копируем данные из packed struct (нельзя брать ссылку на поле packed)
    let rev = rsdp.revision;
    let xsdt_addr = rsdp.xsdt_address;
    let rsdt_addr = rsdp.rsdt_address;

    // XSDT (ACPI 2.0+, revision >= 2) или RSDT (ACPI 1.0)
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);

    let table_addr = if rev >= 2 && xsdt_addr != 0 {
        crate::println!("[ACPI] RSDP rev={}, XSDT={:#x}", rev, xsdt_addr);
        xsdt_addr
    } else if rsdt_addr != 0 {
        crate::println!("[ACPI] RSDP rev={}, RSDT={:#x}", rev, rsdt_addr as u64);
        rsdt_addr as u64
    } else {
        crate::println!("[ACPI] RSDP: neither XSDT nor RSDT is available");
        return;
    };

    let tables = sdt::enumerate_xsdt(table_addr);
    crate::println!("[ACPI] XSDT contains {} tables", tables.len());

    let mut info = AcpiInfo {
        lapic_addr: None,
        io_apic_addr: None,
        io_apic_id: None,
        iso: Vec::new(),
        mcfg_base: None,
        hpet_base: None,
        hpet_comparator_count: None,
        boot_arch_flags: None,
    };

    for &table_phys in &tables {
        // Читаем сигнатуру таблицы через HHDM
        let table_virt = hhdm as usize + table_phys;
        let header = unsafe { &*(table_virt as *const sdt::SdtHeader) };
        let sig = &header.signature;

        match sig {
            b"APIC" => {
                crate::println!("[ACPI] MADT (APIC) found");
                if let Some(madt_info) = madt::parse(table_phys) {
                    info.lapic_addr = Some(madt_info.lapic_addr);
                    info.io_apic_addr = madt_info.io_apic_addr;
                    info.io_apic_id = madt_info.io_apic_id;
                    info.iso = madt_info.iso;
                    crate::println!(
                        "[ACPI]   LAPIC: {:#x}, IOAPIC: {:?} (id={:?}), ISO entries: {}",
                        info.lapic_addr.unwrap_or(0),
                        info.io_apic_addr,
                        info.io_apic_id,
                        info.iso.len()
                    );
                }
            }
            b"FACP" => {
                // IA-PC Boot Architecture Flags: offset 109, bit 1 = 8042.
                if header.length as usize >= 111 {
                    let flags = unsafe {
                        core::ptr::read_unaligned((table_virt + 109) as *const u16)
                    };
                    info.boot_arch_flags = Some(flags);
                    crate::println!(
                        "[ACPI] FADT boot architecture flags: {:#06x} (8042: {})",
                        flags,
                        if flags & 0x0002 != 0 { "yes" } else { "no" }
                    );
                }
            }
            b"MCFG" => {
                crate::println!("[ACPI] MCFG (PCIe) found");
                if let Some(base) = mcfg::parse(table_phys) {
                    info.mcfg_base = Some(base);
                    crate::println!("[ACPI]   MMCONFIG base: {:#x}", base);
                }
            }
            b"HPET" => {
                crate::println!("[ACPI] HPET found");
                if let Some((base, count)) = hpet::parse(table_phys) {
                    info.hpet_base = Some(base);
                    info.hpet_comparator_count = Some(count);
                    crate::println!(
                        "[ACPI]   HPET base: {:#x}, comparators: {}",
                        base,
                        count
                    );
                }
            }
            _ => {
                // Пропускаем неизвестные таблицы
            }
        }
    }

    *ACPI_INFO.lock() = Some(info);
    crate::println!("[ACPI] Initialization complete");
}
