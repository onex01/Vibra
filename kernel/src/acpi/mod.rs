// ACPI — Advanced Configuration and Power Interface
//
// Модуль парсит ключевые ACPI-таблицы: MADT (APIC), MCFG (PCIe ECAM),
// HPET (таймер). RSDP и XSDT используются как точка входа.
//
// Инициализация вызывается после memory::init().

pub mod rsdp;
pub mod sdt;
pub mod smbios;
pub mod madt;
pub mod mcfg;
pub mod hpet;
pub mod power;

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
    /// FADT CENTURY (offset 108) — индекс регистра века в CMOS.
    pub rtc_century_index: Option<u8>,
    /// FADT PM1a_CNT_BLK (offset 64) — порт управления питанием.
    pub pm1a_cnt_blk: Option<u32>,
    /// FADT PM1b_CNT_BLK (offset 68) — второй чипсет, если есть.
    pub pm1b_cnt_blk: Option<u32>,
    /// FADT DSDT (offset 40) — физический адрес таблицы с AML-кодом.
    pub dsdt_phys: Option<u64>,
    /// Значение SLP_TYP из пакета `_S5` в DSDT (обычно 5).
    ///
    /// Берётся из AML, а не из константы: на части платформ `_S5` объявлен
    /// с другим типом сна, и жёстко зашитая пятёрка там просто не выключает
    /// машину.
    pub s5_slp_typ: Option<u16>,
    /// FADT SMI_CMD (offset 48) — нужен для legacy SMI-выключения.
    pub smi_cmd: Option<u32>,
    /// FADT ACPI_ENABLE (offset 52) — значение, включающее ACPI через SMI.
    pub acpi_enable: Option<u8>,
    /// FADT RESET_REG (offset 116) — generic address для сброса машины.
    pub reset_reg: Option<ResetRegister>,
    /// FADT RESET_VALUE (offset 128).
    pub reset_value: Option<u8>,
}

/// Generic Address Structure из FADT для RESET_REG.
#[derive(Clone, Copy, Debug)]
pub struct ResetRegister {
    pub address_space: u8,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub address: u64,
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
        rtc_century_index: None,
        pm1a_cnt_blk: None,
        pm1b_cnt_blk: None,
        dsdt_phys: None,
        s5_slp_typ: None,
        smi_cmd: None,
        acpi_enable: None,
        reset_reg: None,
        reset_value: None,
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
                // CENTURY: offset 108, индекс CMOS-регистра века (0 = нет).
                if header.length as usize >= 109 {
                    let century = unsafe {
                        core::ptr::read_unaligned((table_virt + 108) as *const u8)
                    };
                    if century != 0 && century < 0x80 {
                        info.rtc_century_index = Some(century);
                        crate::println!("[ACPI] FADT RTC century register: {:#04x}", century);
                    }
                }

                // Поля, нужные для выключения питания (см. acpi/power.rs).
                // Смещения фиксированы спецификацией ACPI и не зависят от
                // длины таблицы, но читаем только то, что реально влезает:
                // урезанный FADT встречается на старых машинах.
                if header.length as usize >= 44 {
                    let dsdt = unsafe {
                        core::ptr::read_unaligned((table_virt + 40) as *const u32)
                    } as u64;
                    if dsdt != 0 {
                        info.dsdt_phys = Some(dsdt);
                    }
                }
                if header.length as usize >= 70 {
                    let pm1a = unsafe {
                        core::ptr::read_unaligned((table_virt + 64) as *const u32)
                    };
                    let pm1b = unsafe {
                        core::ptr::read_unaligned((table_virt + 68) as *const u32)
                    };
                    if pm1a != 0 {
                        info.pm1a_cnt_blk = Some(pm1a);
                    }
                    if pm1b != 0 {
                        info.pm1b_cnt_blk = Some(pm1b);
                    }
                    crate::println!(
                        "[ACPI] FADT PM1a_CNT={:#06x}, PM1b_CNT={}",
                        pm1a,
                        match pm1b {
                            0 => alloc::string::String::from("none"),
                            port => alloc::format!("{:#06x}", port),
                        }
                    );
                }
                // SMI_CMD (48) + ACPI_ENABLE (52): legacy-путь включения ACPI.
                if header.length as usize >= 53 {
                    let smi = unsafe {
                        core::ptr::read_unaligned((table_virt + 48) as *const u32)
                    };
                    let enable = unsafe { core::ptr::read_unaligned((table_virt + 52) as *const u8) };
                    if smi != 0 {
                        info.smi_cmd = Some(smi);
                        info.acpi_enable = Some(enable);
                    }
                }
                // RESET_REG (116..128) + RESET_VALUE (128) — современный
                // способ перезагрузки из FADT. На реальном железе он
                // надёжнее захардкоженных портов 0x92/0xCF9.
                if header.length as usize >= 129 {
                    let space = unsafe { core::ptr::read_unaligned((table_virt + 116) as *const u8) };
                    let width = unsafe { core::ptr::read_unaligned((table_virt + 117) as *const u8) };
                    let offset = unsafe { core::ptr::read_unaligned((table_virt + 118) as *const u8) };
                    let address = unsafe {
                        core::ptr::read_unaligned((table_virt + 120) as *const u64)
                    };
                    let value = unsafe { core::ptr::read_unaligned((table_virt + 128) as *const u8) };
                    if width != 0 && address != 0 {
                        info.reset_reg = Some(ResetRegister {
                            address_space: space,
                            bit_width: width,
                            bit_offset: offset,
                            address,
                        });
                        info.reset_value = Some(value);
                        crate::println!(
                            "[ACPI] FADT reset_reg: space={}, width={}, addr={:#x}, value={:#x}",
                            space, width, address, value
                        );
                    }
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

    // SLP_TYP для выключения берём из AML-пакета `_S5` в DSDT.
    // Без этого приходится угадывать значение, и на платформах с другим
    // SLP_TYP запись в PM1a_CNT просто игнорируется.
    if let Some(dsdt) = info.dsdt_phys {
        match power::parse_s5_slp_typ(dsdt) {
            Some(slp_typ) => {
                info.s5_slp_typ = Some(slp_typ);
                crate::println!("[ACPI] DSDT _S5 SLP_TYP = {}", slp_typ);
            }
            None => {
                crate::println!("[ACPI] DSDT _S5 not found; poweroff will fall back");
            }
        }
    }

    *ACPI_INFO.lock() = Some(info);
    crate::println!("[ACPI] Initialization complete");
}

/// Список таблиц из XSDT/RSDT: `(сигнатура, физический адрес, длина)`
/// в порядке перечисления. Нужен `diag hw`: какие таблицы реально есть на
/// машине (ECDT, DSDT, SSDT, SRAT... ), а не только те, что мы парсим.
pub fn table_list() -> Vec<([u8; 4], u64, u32)> {
    let mut result = Vec::new();
    let Some(rsdp_virt) = rsdp::find_rsdp() else {
        return result;
    };
    let rsdp = unsafe { &*(rsdp_virt as *const rsdp::Rsdp) };
    let revision = rsdp.revision;
    let xsdt = rsdp.xsdt_address;
    let rsdt = rsdp.rsdt_address;

    let table = if revision >= 2 && xsdt != 0 {
        xsdt
    } else {
        rsdt as u64
    };
    if table == 0 {
        return result;
    }

    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed) as usize;

    for phys in sdt::enumerate_xsdt(table) {
        let header = unsafe { &*((hhdm + phys) as *const sdt::SdtHeader) };
        let signature = header.signature;
        let length = header.length;
        result.push((signature, phys as u64, length));
    }
    result
}
