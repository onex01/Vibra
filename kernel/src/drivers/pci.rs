// PCI Bus — обнаружение и конфигурация устройств на PCI шине.
//
// PCI конфигурационное пространство:
//   Адресный порт: 0xCF8 (32-bit: bus:8 | dev:5 | func:8 | reg:8 | enable:1)
//   Данный порт:   0xCFC (32-bit data read/write)

use crate::println;
use alloc::string::String;
use alloc::format;
use alloc::vec::Vec;
use spin::Mutex;

const CONFIG_ADDR: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

const MAX_BUS: u8 = 255;
const MAX_DEV: u8 = 32;
const MAX_FUNC: u8 = 8;

#[inline]
unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    core::arch::asm!("in eax, dx", out("eax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline]
unsafe fn outw(port: u16, val: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
}

/// Записать 32-битное значение в PCI конфигурационное пространство.
/// offset — регистр (4-байтное выравнивание).
unsafe fn pci_write_u32(bus: u8, dev: u8, func: u8, offset: u8, value: u32) {
    let addr = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((offset & 0xFC) as u32);
    outl(CONFIG_ADDR, addr);
    outl(CONFIG_DATA, value);
}

/// Прочитать 32-битное значение из PCI конфигурационного пространства.
/// offset — регистр (4-байтное выравнивание).
unsafe fn pci_read_u32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let addr = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((offset & 0xFC) as u32);
    outl(CONFIG_ADDR, addr);
    inl(CONFIG_DATA)
}

unsafe fn pci_read_u16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    let val = pci_read_u32(bus, dev, func, offset & 0xFC);
    ((val >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

unsafe fn pci_write_u16(bus: u8, dev: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & 0xFC;
    let shift = (offset & 2) * 8;
    let old = pci_read_u32(bus, dev, func, aligned);
    let mask = !(0xFFFFu32 << shift);
    pci_write_u32(bus, dev, func, aligned, (old & mask) | ((value as u32) << shift));
}

unsafe fn pci_read_u8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    let val = pci_read_u32(bus, dev, func, offset & 0xFC);
    ((val >> ((offset & 3) * 8)) & 0xFF) as u8
}

/// Приводит memory BAR0 в низкое MMIO-окно.
///
/// OVMF размещает 64-битный BAR xHCI на `0x8000000000` (512 ГБ), а HHDM
/// покрывает только физическую память — обращение по такому адресу даёт
/// page fault. Поэтому BAR выше 4 ГБ переносим в свободное окно сразу за
/// самым высоким уже назначенным BAR.
unsafe fn assign_bar0_if_needed(device: &mut PciDevice, next_mmio: &mut u64) {
    // Только обычные устройства: у PCI-мостов (header type 1) offset 0x10 —
    // это Base/Limit, а не BAR, и писать туда нельзя.
    if device.header_type & 0x7F != 0 {
        return;
    }

    let low = device.bar0;
    if low == 0xFFFF_FFFF || low & 1 != 0 {
        return; // устройства нет или это I/O BAR
    }

    let is_64 = low & 0x6 == 0x4;
    let high = if is_64 { device.bar1 } else { 0 };
    let address = ((high as u64) << 32) | (low as u64 & 0xFFFF_FFF0);

    // Уже в низком MMIO-окне — не трогаем.
    if address != 0 && address < 0xFEC0_0000 {
        return;
    }

    let bus = device.bus;
    let dev = device.dev;
    let func = device.func;

    // На время изменения BAR выключаем memory decoding.
    let command = pci_read_u16(bus, dev, func, 0x04);
    pci_write_u16(bus, dev, func, 0x04, command & !0x0002);

    // Размер: пишем все единицы (для 64-битного BAR — в оба dword).
    pci_write_u32(bus, dev, func, 0x10, 0xFFFF_FFFF);
    if is_64 {
        pci_write_u32(bus, dev, func, 0x14, 0xFFFF_FFFF);
    }
    let mask_low = pci_read_u32(bus, dev, func, 0x10);
    let mask_high = if is_64 {
        pci_read_u32(bus, dev, func, 0x14)
    } else {
        0
    };

    // Возвращаем исходные значения.
    pci_write_u32(bus, dev, func, 0x10, low);
    if is_64 {
        pci_write_u32(bus, dev, func, 0x14, high);
    }

    let mask = ((mask_high as u64) << 32) | (mask_low as u64 & 0xFFFF_FFF0);
    if mask == 0 || mask == u64::MAX {
        pci_write_u16(bus, dev, func, 0x04, command);
        return;
    }
    let size = (!mask).wrapping_add(1);
    if size == 0 || size > (64 << 20) || !size.is_power_of_two() {
        pci_write_u16(bus, dev, func, 0x04, command);
        return;
    }

    let base = (*next_mmio + size - 1) & !(size - 1);
    if base >= 0xFEC0_0000 || base + size > 0xFEC0_0000 {
        pci_write_u16(bus, dev, func, 0x04, command);
        return;
    }

    pci_write_u32(bus, dev, func, 0x10, base as u32);
    if is_64 {
        pci_write_u32(bus, dev, func, 0x14, (base >> 32) as u32);
    }
    *next_mmio = base + size;

    // Memory Space + Bus Master
    pci_write_u16(bus, dev, func, 0x04, command | 0x0002 | 0x0004);

    device.bar0 = base as u32 | (low & 0xF);
    device.bar1 = (base >> 32) as u32;
    device.bar_phys[0] = base;
    device.bar_phys[1] = 0;

    println!(
        "[PCI] BAR0 {:#x} -> {:#x} ({} KiB) for {:04x}:{:04x}",
        address,
        base,
        size / 1024,
        device.vendor_id,
        device.device_id
    );
}

/// PCI Device Descriptor
#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
    pub bar0: u32,
    pub bar1: u32,
    pub bar2: u32,
    pub bar3: u32,
    pub bar4: u32,
    pub bar5: u32,
    pub bar_phys: [u64; 6],
    pub irq: u8,
}

impl PciDevice {
    pub fn class_name(&self) -> &'static str {
        match self.class {
            0x00 => "Unclassified",
            0x01 => "Mass Storage",
            0x02 => "Network",
            0x03 => "Display",
            0x04 => "Multimedia",
            0x05 => "Memory",
            0x06 => "Bridge",
            0x07 => "Communication",
            0x08 => "System",
            0x09 => "Input",
            0x0A => "Docking",
            0x0B => "Processor",
            0x0C => "Serial Bus",
            0x0D => "Wireless",
            0x0E => "Intelligent I/O",
            0x0F => "Satellite",
            0x10 => "Cryptographic",
            0x11 => "Signal Processing",
            0x12 => "Processing Accelerator",
            0x13 => "Non-Essential Instrumentation",
            _ => "Other",
        }
    }

    pub fn subclass_name(&self) -> &'static str {
        match (self.class, self.subclass) {
            (0x01, 0x00) => "SCSI",
            (0x01, 0x01) => "IDE",
            (0x01, 0x06) => "SATA (AHCI)",
            (0x01, 0x08) => "NVMe",
            (0x06, 0x00) => "Host Bridge",
            (0x06, 0x01) => "ISA Bridge",
            (0x06, 0x04) => "PCI-to-PCI Bridge",
            (0x03, 0x00) => "VGA",
            (0x02, 0x00) => "Ethernet",
            (0x0C, 0x03) => "USB",
            _ => "",
        }
    }

    /// BAR (Base Address Register) — физический адрес устройства.
    /// Для 64-битных memory BAR используем уже собранный `bar_phys`.
    pub fn bar0_phys(&self) -> u64 {
        if self.bar_phys[0] != 0 {
            self.bar_phys[0]
        } else {
            self.bar0 as u64 & 0xFFFF_FFF0
        }
    }

    pub fn is_io_bar(&self, bar_num: usize) -> bool {
        let bar = match bar_num {
            0 => self.bar0,
            1 => self.bar1,
            2 => self.bar2,
            3 => self.bar3,
            4 => self.bar4,
            5 => self.bar5,
            _ => return false,
        };
        bar & 1 != 0 // bit 0 = 1 → I/O, bit 0 = 0 → memory
    }

    /// Прочитать физический адрес BAR через конфигурационное пространство PCI.
    /// Поддерживает 32-бит и 64-бит memory BAR, а также I/O BAR.
    pub fn bar_phys_from_config(&self, bar_num: u8) -> u64 {
        unsafe {
            let low = pci_read_u32(self.bus, self.dev, self.func, 0x10 + bar_num * 4);
            if low & 1 == 0 {
                // Memory BAR
                let bar_type = (low >> 1) & 0x3;
                match bar_type {
                    0 => low as u64 & 0xFFFFFFF0, // 32-bit
                    2 => {
                        // 64-bit: объединяем BAR[n] + BAR[n+1]
                        let high = pci_read_u32(self.bus, self.dev, self.func, 0x10 + (bar_num + 1) * 4);
                        ((high as u64) << 32) | ((low as u64) & 0xFFFFFFF0)
                    }
                    _ => low as u64 & 0xFFFFFFF0, // 16-bit или неизвестный
                }
            } else {
                // I/O BAR
                (low & !0x3) as u64
            }
        }
    }
}

static PCI_DEVICES: Mutex<Vec<PciDevice>> = Mutex::new(Vec::new());

/// Сканировать PCI шину и найти все устройства.
///
/// Оптимизации (на реальном железе порт I/O ~1 мкс на обращение, полный обход
/// 256×32×8 — это сотни тысяч обращений и секунды простоя):
///   1. Пустая шина (ни одного устройства) означает, что дальше шин нет —
///      выходим из цикла, а не сканируем все 256.
///   2. Заголовок читается 3 словами (0x00, 0x08, 0x0C) вместо 6 полей.
///   3. BAR'ы берутся из уже прочитанных слов, без повторного чтения.
pub fn init() {
    println!("[PCI] Scanning PCI bus...");

    let mut devices = PCI_DEVICES.lock();
    let mut count = 0u32;
    let mut buses_scanned = 0u32;
    let mut empty_streak = 0u32;

    for bus in 0..=MAX_BUS {
        let mut bus_has_device = false;
        buses_scanned += 1;

        for dev in 0..MAX_DEV {
            for func in 0..MAX_FUNC {
                unsafe {
                    // vendor(0x00) + device(0x02) одним 32-битным чтением.
                    let id_reg = pci_read_u32(bus, dev, func, 0x00);
                    let vendor = (id_reg & 0xFFFF) as u16;
                    if vendor == 0xFFFF {
                        // No device at this slot
                        if func == 0 { break; } // Skip rest of functions
                        continue;
                    }

                    bus_has_device = true;
                    let device_id = (id_reg >> 16) as u16;

                    let class_reg = pci_read_u32(bus, dev, func, 0x08);
                    // 0x0C: cache line, latency, header type (0x0E), BIST.
                    let header_reg = pci_read_u32(bus, dev, func, 0x0C);
                    let header_type = ((header_reg >> 16) & 0xFF) as u8;

                    let class = ((class_reg >> 24) & 0xFF) as u8;
                    let subclass = ((class_reg >> 16) & 0xFF) as u8;
                    let prog_if = ((class_reg >> 8) & 0xFF) as u8;

                    let bars = [
                        pci_read_u32(bus, dev, func, 0x10),
                        pci_read_u32(bus, dev, func, 0x14),
                        pci_read_u32(bus, dev, func, 0x18),
                        pci_read_u32(bus, dev, func, 0x1C),
                        pci_read_u32(bus, dev, func, 0x20),
                        pci_read_u32(bus, dev, func, 0x24),
                    ];
                    let [bar0, bar1, bar2, bar3, bar4, bar5] = bars;

                    let irq = pci_read_u8(bus, dev, func, 0x3C);

                    let mut device = PciDevice {
                        bus, dev, func,
                        vendor_id: vendor,
                        device_id,
                        class, subclass, prog_if,
                        header_type,
                        bar0, bar1, bar2, bar3, bar4, bar5,
                        bar_phys: [0; 6],
                        irq,
                    };

                    // Вычисляем физические адреса всех BAR из уже прочитанных слов.
                    for i in 0..6usize {
                        let raw = bars[i];
                        if raw == 0 || raw == 0xFFFFFFFF {
                            continue;
                        }
                        if raw & 1 == 0 {
                            // Memory BAR
                            let bar_type = (raw >> 1) & 0x3;
                            if bar_type == 2 && i < 5 {
                                // 64-bit: объединяем BAR[n] + BAR[n+1]
                                device.bar_phys[i] = ((bars[i + 1] as u64) << 32) | ((raw as u64) & 0xFFFFFFF0);
                            } else {
                                device.bar_phys[i] = raw as u64 & 0xFFFFFFF0;
                            }
                        } else {
                            // I/O BAR
                            device.bar_phys[i] = (raw & !0x3) as u64;
                        }
                    }

                    devices.push(device);
                    count += 1;

                    // Если multi-function device (header_type bit 7) — читаем все 8 func
                    if func == 0 && header_type & 0x80 == 0 {
                        break; // Single function device
                    }
                }
            }
        }

        // Пустая шина => следующих шин почти наверняка нет (стандартная
        // эвристика PCI/PCIe: нумерация начинается с 0 и не имеет длинных
        // дырок). Выходим после 4 пустых подряд, чтобы не потерять устройства
        // за редким разрывом нумерации. Главное ускорение: 1-3 шины вместо 256.
        if bus_has_device {
            empty_streak = 0;
        } else {
            empty_streak += 1;
            if empty_streak >= 4 {
                break;
            }
        }
    }

    // Второй проход: memory BAR'ы, которые прошивка оставила нулевыми.
    // Без этого xHCI на OVMF вообще не виден (`BAR0 is zero`).
    // Свободный адрес берём сразу за самым высоким уже назначенным BAR,
    // чтобы остаться в том же (заведомо отображённом) MMIO-окне.
    let mut next_mmio = 0x8000_0000u64;
    for device in devices.iter() {
        for bar in device.bar_phys {
            if bar >= next_mmio && bar < 0xFEC0_0000 {
                next_mmio = bar;
            }
        }
    }
    next_mmio = (next_mmio + 0x100000) & !0xFFFFF;
    for device in devices.iter_mut() {
        unsafe { assign_bar0_if_needed(device, &mut next_mmio) };
    }

    println!("[PCI] Found {} devices on {} bus(es)", count, buses_scanned);

    // Выводим список
    for dev in devices.iter() {
        println!("  [{:02X}:{:02X}.{}] {:04X}:{:04X} {} {} {} IRQ={}",
            dev.bus, dev.dev, dev.func,
            dev.vendor_id, dev.device_id,
            dev.class_name(), dev.subclass_name(),
            if dev.bar0 != 0 { format!("BAR0={:#x}", dev.bar0 & 0xFFFFFFF0) } else { String::new() },
            dev.irq,
        );
    }
}

/// Найти устройство по class/subclass
pub fn find_device(class: u8, subclass: u8) -> Option<PciDevice> {
    let devices = PCI_DEVICES.lock();
    devices.iter().find(|d| d.class == class && d.subclass == subclass).copied()
}

/// Найти устройство по vendor_id/device_id
pub fn find_device_by_id(vendor: u16, device: u16) -> Option<PciDevice> {
    let devices = PCI_DEVICES.lock();
    devices.iter().find(|d| d.vendor_id == vendor && d.device_id == device).copied()
}

/// Записать 16-битное значение в PCI конфигурационное пространство
pub unsafe fn pci_write_u16_config(bus: u8, dev: u8, func: u8, offset: u8, value: u16) {
    pci_write_u32(bus, dev, func, offset & 0xFC, value as u32);
}

/// Прочитать 8-битное значение из PCI конфигурационного пространства
pub unsafe fn pci_read_u8_config(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    pci_read_u8(bus, dev, func, offset)
}

/// Прочитать 16-битное значение из PCI конфигурационного пространства
pub unsafe fn pci_read_u16_config(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    pci_read_u16(bus, dev, func, offset)
}

/// Прочитать 32-битное значение из PCI конфигурационного пространства
pub unsafe fn pci_read_u32_config(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    pci_read_u32(bus, dev, func, offset)
}

/// Включить Bus Master (bit 2) для PCI устройства — необходимо для DMA
pub unsafe fn enable_bus_master(bus: u8, dev: u8, func: u8) {
    let mut command = pci_read_u32(bus, dev, func, 0x04);
    command |= (1 << 2) | (1 << 1); // Bus Master + Memory Space
    pci_write_u32(bus, dev, func, 0x04, command);
}

/// Найти все устройства по class/subclass
pub fn find_all_devices_by_class(class: u8, subclass: u8) -> Vec<PciDevice> {
    let devices = PCI_DEVICES.lock();
    devices.iter().filter(|d| d.class == class && d.subclass == subclass).copied().collect()
}

/// Количество найденных устройств
pub fn device_count() -> usize {
    PCI_DEVICES.lock().len()
}

/// Прочитать физический адрес BAR устройства через конфигурационное пространство.
/// Поддерживает 32-бит, 64-бит memory BAR и I/O BAR.
pub fn bar_phys(bus: u8, dev: u8, func: u8, bar_num: u8) -> u64 {
    unsafe {
        let low = pci_read_u32(bus, dev, func, 0x10 + bar_num * 4);
        if low & 1 == 0 {
            // Memory BAR: биты [3:1] указывают тип
            let bar_type = (low >> 1) & 0x3;
            match bar_type {
                0 => low as u64 & 0xFFFFFFF0, // 32-bit
                2 => {
                    // 64-bit: BAR[n] (low) + BAR[n+1] (high)
                    let high = pci_read_u32(bus, dev, func, 0x10 + (bar_num + 1) * 4);
                    ((high as u64) << 32) | ((low as u64) & 0xFFFFFFF0)
                }
                _ => low as u64 & 0xFFFFFFF0, // 16-bit или неизвестный
            }
        } else {
            // I/O BAR
            (low & !0x3) as u64
        }
    }
}
