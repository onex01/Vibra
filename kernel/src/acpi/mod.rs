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
    pub iso: Vec<(u8, u32)>,
    pub mcfg_base: Option<u64>,
    pub hpet_base: Option<u64>,
    pub hpet_comparator_count: Option<u8>,
}

pub static ACPI_INFO: Mutex<Option<AcpiInfo>> = Mutex::new(None);

/// Получить текущий AcpiInfo (блокировка).
pub fn get() -> spin::MutexGuard<'static, Option<AcpiInfo>> {
    ACPI_INFO.lock()
}

/// Инициализация ACPI: RSDP → XSDT → MADT/MCFG/HPET.
pub fn init() {
    crate::println!("[ACPI] Поиск RSDP...");

    let rsdp_virt = match rsdp::find_rsdp() {
        Some(v) => v,
        None => {
            crate::println!("[ACPI] RSDP не найден, ACPI недоступен");
            return;
        }
    };

    crate::println!("[ACPI] RSDP найден по адресу {:#x}", rsdp_virt);

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
        crate::println!("[ACPI] RSDP: ни XSDT, ни RSDT недоступны");
        return;
    };

    let tables = sdt::enumerate_xsdt(table_addr);
    crate::println!("[ACPI] XSDT содержит {} подтаблиц", tables.len());

    let mut info = AcpiInfo {
        lapic_addr: None,
        io_apic_addr: None,
        io_apic_id: None,
        iso: Vec::new(),
        mcfg_base: None,
        hpet_base: None,
        hpet_comparator_count: None,
    };

    for &table_phys in &tables {
        // Читаем сигнатуру таблицы через HHDM
        let table_virt = hhdm as usize + table_phys;
        let header = unsafe { &*(table_virt as *const sdt::SdtHeader) };
        let sig = &header.signature;

        match sig {
            b"APIC" => {
                crate::println!("[ACPI] Найдена MADT (APIC)");
                if let Some(madt_info) = madt::parse(table_phys) {
                    info.lapic_addr = Some(madt_info.lapic_addr);
                    info.io_apic_addr = madt_info.io_apic_addr;
                    info.io_apic_id = madt_info.io_apic_id;
                    info.iso = madt_info.iso;
                    crate::println!(
                        "[ACPI]   LAPIC: {:#x}, IOAPIC: {:?} (id={:?}), ISO: {} записей",
                        info.lapic_addr.unwrap_or(0),
                        info.io_apic_addr,
                        info.io_apic_id,
                        info.iso.len()
                    );
                }
            }
            b"MCFG" => {
                crate::println!("[ACPI] Найдена MCFG (PCIe)");
                if let Some(base) = mcfg::parse(table_phys) {
                    info.mcfg_base = Some(base);
                    crate::println!("[ACPI]   MMCONFIG base: {:#x}", base);
                }
            }
            b"HPET" => {
                crate::println!("[ACPI] Найдена HPET");
                if let Some((base, count)) = hpet::parse(table_phys) {
                    info.hpet_base = Some(base);
                    info.hpet_comparator_count = Some(count);
                    crate::println!(
                        "[ACPI]   HPET base: {:#x}, компариторов: {}",
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
    crate::println!("[ACPI] Инициализация завершена");
}
