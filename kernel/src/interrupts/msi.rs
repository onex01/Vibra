// MSI/MSI-X — поддержка Message Signaled Interrupts для PCI устройств.
//
// MSI (Message Signaled Interrupts):
//   - Устройство пишет 32-битное сообщение в специальный адрес APIC
//   - Адрес формируется: 0xFEE0_0000 | (destination << 12) | (redir_hint << 3)
//   - Данные: (delivery_mode << 8) | vector
//   - Контролируется через PCI Capability List (ID = 0x05)
//
// MSI-X (MSI eXtended):
//   - Таблица векторов в MMIO-пространстве (BAR + offset)
//   - Поддерживает больше векторов и пер-векторную маску
//   - Контролируется через PCI Capability List (ID = 0x11)
//
// Используется для: NVMe, AHCI, VirtIO, Network, USB xHCI и др.

use crate::println;
use crate::drivers::pci::PciDevice;
use core::sync::atomic::{AtomicU8, Ordering};

// ======================== PCI Capability IDs ========================

const PCI_CAP_MSI: u8 = 0x05;
const PCI_CAP_MSIX: u8 = 0x11;
const PCI_CAP_PTR_REG: u8 = 0x34; // Смещение указателя на capabilities

// ======================== MSI Message Control ========================

const MSI_MC_ENABLE: u16 = 1 << 0;
const MSI_MC_64BIT: u16 = 1 << 7;
const MSI_MC_MULTIPLE_MASK: u16 = 0x0E;

// ======================== MSI-X Message Control ========================

const MSIX_MC_ENABLE: u16 = 1 << 15;
const MSIX_TC_MASK: u16 = 0x07FF;

// ======================== APIC Constants ========================

const APIC_BASE_ADDR: u32 = 0xFEE0_0000;
const DELIVERY_FIXED: u32 = 0;
const DEST_PHYSICAL: u32 = 0;

// ======================== Vector Counter ========================

/// Глобальный счётчик векторов для выделения
static NEXT_VECTOR: AtomicU8 = AtomicU8::new(64); // Начинаем с 64 (выше PIC/APIC range)

/// Выделить следующий свободный вектор прерывания
pub fn allocate_vector() -> u8 {
    let v = NEXT_VECTOR.fetch_add(1, Ordering::Relaxed);
    if v == 0 {
        // Переполнение: начинаем заново
        NEXT_VECTOR.store(65, Ordering::Relaxed);
        64
    } else {
        v
    }
}

// ======================== Low-Level PCI Config ========================

unsafe fn pci_config_read_u8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    super::super::drivers::pci::pci_read_u8_config(bus, dev, func, offset)
}

unsafe fn pci_config_read_u16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    super::super::drivers::pci::pci_read_u16_config(bus, dev, func, offset)
}

unsafe fn pci_config_read_u32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    super::super::drivers::pci::pci_read_u32_config(bus, dev, func, offset)
}

unsafe fn pci_config_write_u16(bus: u8, dev: u8, func: u8, offset: u8, value: u16) {
    super::super::drivers::pci::pci_write_u16_config(bus, dev, func, offset, value);
}

// ======================== Capability List Traversal ========================

/// Результат поиска MSI capability
pub enum MsiCap {
    /// MSI: capability offset, 64-bit capable, max vectors
    Msi { offset: u8, is_64bit: bool, max_vectors: u8 },
    /// MSI-X: capability offset, table offset, table BIR, PBA offset, PBA BIR, table size
    Msix { offset: u8, table_offset: u32, table_bir: u8, pba_offset: u32, pba_bir: u8, table_size: u16 },
}

/// Прочитать capabilities pointer из PCI config space
fn get_cap_ptr(bus: u8, dev: u8, func: u8) -> u8 {
    unsafe {
        let status_reg = pci_config_read_u16(bus, dev, func, 0x04);
        if status_reg & (1 << 4) == 0 {
            // Capability List не поддерживается
            return 0;
        }
        pci_config_read_u8(bus, dev, func, PCI_CAP_PTR_REG) & 0xFC
    }
}

/// Ищем MSI и MSI-X capabilities в PCI config space
pub fn find_msi_capabilities(bus: u8, dev: u8, func: u8) -> (Option<MsiCap>, Option<MsiCap>) {
    let mut msi: Option<MsiCap> = None;
    let mut msix: Option<MsiCap> = None;

    let mut cap_ptr = get_cap_ptr(bus, dev, func);
    let mut iterations = 0u8;

    while cap_ptr != 0 && iterations < 64 {
        unsafe {
            let cap_id = pci_config_read_u8(bus, dev, func, cap_ptr);
            let next_ptr = pci_config_read_u8(bus, dev, func, cap_ptr + 1) & 0xFC;

            match cap_id {
                PCI_CAP_MSI => {
                    let msg_control = pci_config_read_u16(bus, dev, func, cap_ptr + 2);
                    let is_64bit = msg_control & MSI_MC_64BIT != 0;
                    let multi_mask = (msg_control & MSI_MC_MULTIPLE_MASK) >> 1;
                    let max_vectors = 1u8 << multi_mask;

                    println!("    [MSI] Найден MSI capability @ 0x{:02x} (64-bit={}, max_vectors={})",
                        cap_ptr, is_64bit, max_vectors);

                    msi = Some(MsiCap::Msi { offset: cap_ptr, is_64bit, max_vectors });
                }
                PCI_CAP_MSIX => {
                    let msg_control = pci_config_read_u16(bus, dev, func, cap_ptr + 2);
                    let table_size_raw = (msg_control & MSIX_TC_MASK) as u16;
                    let table_size = table_size_raw + 1; // 0-based

                    // Table offset + BAR (offset 4)
                    let offset_bir = pci_config_read_u32(bus, dev, func, cap_ptr + 4);
                    let table_bir = (offset_bir & 0x07) as u8;
                    let table_offset = offset_bir & 0xFFFFFFF8;

                    // PBA offset + BAR (offset 8)
                    let pba_bir_off = pci_config_read_u32(bus, dev, func, cap_ptr + 8);
                    let pba_bir = (pba_bir_off & 0x07) as u8;
                    let pba_offset = pba_bir_off & 0xFFFFFFF8;

                    println!("    [MSI-X] Найден MSI-X capability @ 0x{:02x} (table_size={}, BAR{}, offset={:#x})",
                        cap_ptr, table_size, table_bir, table_offset);

                    msix = Some(MsiCap::Msix {
                        offset: cap_ptr,
                        table_offset, table_bir,
                        pba_offset, pba_bir,
                        table_size,
                    });
                }
                _ => {}
            }

            cap_ptr = next_ptr;
            iterations += 1;
        }
    }

    (msi, msix)
}

// ======================== MSI Enable ========================

/// Структура MSI table entry (в MMIO пространстве для MSI-X)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MsixTableEntry {
    pub msg_addr_lo: u32,
    pub msg_addr_hi: u32,
    pub msg_data: u32,
    pub vector_control: u32,
}

/// Настроить и включить MSI для PCI устройства.
/// Возвращает выделенный вектор прерывания.
pub fn enable_msi(bus: u8, dev: u8, func: u8, msi_cap: &MsiCap) -> Option<u8> {
    let (cap_offset, is_64bit) = match msi_cap {
        MsiCap::Msi { offset, is_64bit, .. } => (*offset, *is_64bit),
        _ => {
            println!("    [MSI] ОШИБКА: передан не MSI capability");
            return None;
        }
    };

    let vector = allocate_vector();

    // Формируем Message Address (APIC ICR format)
    // Формат: 0xFEE0_0000 | (destination << 12) | (redir_hint << 3)
    // Используем Physical Destination, Fixed delivery, все CPUs
    let msg_addr = APIC_BASE_ADDR as u64; // shorthand = all including self, physical
    let msg_addr_lo = msg_addr as u32;
    let msg_addr_hi = (msg_addr >> 32) as u32;

    // Message Data: (delivery_mode << 8) | vector
    let msg_data = (DELIVERY_FIXED << 8) | (vector as u32);

    unsafe {
        // Записываем Message Address
        if is_64bit {
            pci_config_write_u16(bus, dev, func, cap_offset + 4, msg_addr_lo as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 6, (msg_addr_lo >> 16) as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 8, msg_addr_hi as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 10, (msg_addr_hi >> 16) as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 12, msg_data as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 14, (msg_data >> 16) as u16);
        } else {
            // 32-bit MSI
            pci_config_write_u16(bus, dev, func, cap_offset + 4, msg_addr_lo as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 6, (msg_addr_lo >> 16) as u16);
            pci_config_write_u16(bus, dev, func, cap_offset + 8, msg_data as u16);
        }

        // Включаем MSI: устанавливаем Enable bit в Message Control
        let mut msg_control = pci_config_read_u16(bus, dev, func, cap_offset + 2);
        msg_control |= MSI_MC_ENABLE;
        pci_config_write_u16(bus, dev, func, cap_offset + 2, msg_control);

        println!("    [MSI] Включён: vector={}, addr={:#x}, data={:#x}",
            vector, msg_addr, msg_data);
    }

    Some(vector)
}

/// Отключить MSI для PCI устройства
pub fn disable_msi(bus: u8, dev: u8, func: u8, cap_offset: u8) {
    unsafe {
        let mut msg_control = pci_config_read_u16(bus, dev, func, cap_offset + 2);
        msg_control &= !MSI_MC_ENABLE;
        pci_config_write_u16(bus, dev, func, cap_offset + 2, msg_control);
        println!("    [MSI] Отключён");
    }
}

// ======================== MSI-X Enable ========================

/// Настроить и включить MSI-X для PCI устройства на указанном векторе.
/// Возвращает выделенный вектор прерывания.
pub fn enable_msix(bus: u8, dev: u8, func: u8, msix_cap: &MsiCap, entry_index: u16) -> Option<u8> {
    let (cap_offset, table_offset, table_bir) = match msix_cap {
        MsiCap::Msix { offset, table_offset, table_bir, .. } => (*offset, *table_offset, *table_bir),
        _ => {
            println!("    [MSI-X] ОШИБКА: передан не MSI-X capability");
            return None;
        }
    };

    let vector = allocate_vector();

    // Физический адрес BAR[table_bir]
    let bar_phys = unsafe {
        super::super::drivers::pci::bar_phys(bus, dev, func, table_bir)
    };
    if bar_phys == 0 {
        println!("    [MSI-X] ОШИБКА: BAR{} = 0", table_bir);
        return None;
    }

    // Адрес записи: hhdm + bar_phys + table_offset + entry_index * 16
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let entry_virt = hhdm + bar_phys + table_offset as u64 + (entry_index as u64) * 16;

    let msg_addr = APIC_BASE_ADDR as u64;
    let msg_data = (DELIVERY_FIXED << 8) | (vector as u32);

    unsafe {
        let entry_base = entry_virt as *mut u8;
        core::ptr::write_volatile(entry_base as *mut u32, msg_addr as u32);
        core::ptr::write_volatile(entry_base.add(4) as *mut u32, (msg_addr >> 32) as u32);
        core::ptr::write_volatile(entry_base.add(8) as *mut u32, msg_data);
        core::ptr::write_volatile(entry_base.add(12) as *mut u32, 0u32); // Unmasked

        // Включаем MSI-X: Global Enable в Message Control
        let mut msg_control = pci_config_read_u16(bus, dev, func, cap_offset + 2);
        msg_control |= MSIX_MC_ENABLE;
        pci_config_write_u16(bus, dev, func, cap_offset + 2, msg_control);

        println!("    [MSI-X] Включён: entry={}, vector={}, addr={:#x}, data={:#x}",
            entry_index, vector, msg_addr, msg_data);
    }

    Some(vector)
}

/// Отключить MSI-X для PCI устройства
pub fn disable_msix(bus: u8, dev: u8, func: u8, cap_offset: u8) {
    unsafe {
        let mut msg_control = pci_config_read_u16(bus, dev, func, cap_offset + 2);
        msg_control &= !MSIX_MC_ENABLE;
        pci_config_write_u16(bus, dev, func, cap_offset + 2, msg_control);
        println!("    [MSI-X] Отключён");
    }
}

/// Полный процесс: найти capability и включить MSI/MSI-X для устройства.
/// Возвращает (vector, is_msix).
pub fn try_enable_msi(device: &PciDevice) -> Option<(u8, bool)> {
    let (msi, msix) = find_msi_capabilities(device.bus, device.dev, device.func);

    // Предпочитаем MSI-X (гибкость, больше векторов)
    if let Some(ref msix_cap) = msix {
        if let Some(vector) = enable_msix(device.bus, device.dev, device.func, msix_cap, 0) {
            return Some((vector, true));
        }
    }

    // Fallback на MSI
    if let Some(ref msi_cap) = msi {
        if let Some(vector) = enable_msi(device.bus, device.dev, device.func, msi_cap) {
            return Some((vector, false));
        }
    }

    None
}

/// Инициализация MSI/MSI-X подсистемы (пока только логирование)
pub fn init() {
    println!("[MSI] Подсистема MSI/MSI-X инициализирована");
}
