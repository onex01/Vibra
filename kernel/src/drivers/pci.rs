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

unsafe fn pci_read_u8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    let val = pci_read_u32(bus, dev, func, offset & 0xFC);
    ((val >> ((offset & 3) * 8)) & 0xFF) as u8
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

    /// BAR (Base Address Register) — физический адрес устройства
    pub fn bar0_phys(&self) -> u64 {
        self.bar0 as u64 & 0xFFFFFFF0
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
        bar & 1 == 0 // bit 0 = 0 → memory, bit 0 = 1 → I/O
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

/// Сканировать PCI шину и найти все устройства
pub fn init() {
    println!("[PCI] Scanning PCI bus...");

    let mut devices = PCI_DEVICES.lock();
    let mut count = 0u32;

    for bus in 0..=MAX_BUS {
        for dev in 0..MAX_DEV {
            for func in 0..MAX_FUNC {
                unsafe {
                    let vendor = pci_read_u16(bus, dev, func, 0x00);
                    if vendor == 0xFFFF {
                        // No device at this slot
                        if func == 0 { break; } // Skip rest of functions
                        continue;
                    }

                    let device_id = pci_read_u16(bus, dev, func, 0x02);
                    let class_reg = pci_read_u32(bus, dev, func, 0x08);
                    let header_type = pci_read_u8(bus, dev, func, 0x0E);

                    let class = ((class_reg >> 24) & 0xFF) as u8;
                    let subclass = ((class_reg >> 16) & 0xFF) as u8;
                    let prog_if = ((class_reg >> 8) & 0xFF) as u8;

                    let bar0 = pci_read_u32(bus, dev, func, 0x10);
                    let bar1 = pci_read_u32(bus, dev, func, 0x14);
                    let bar2 = pci_read_u32(bus, dev, func, 0x18);
                    let bar3 = pci_read_u32(bus, dev, func, 0x1C);
                    let bar4 = pci_read_u32(bus, dev, func, 0x20);
                    let bar5 = pci_read_u32(bus, dev, func, 0x24);

                    let irq_reg = pci_read_u32(bus, dev, func, 0x0C);
                    let irq = pci_read_u8(bus, dev, func, 0x3C);

                    let device = PciDevice {
                        bus, dev, func,
                        vendor_id: vendor,
                        device_id,
                        class, subclass, prog_if,
                        header_type,
                        bar0, bar1, bar2, bar3, bar4, bar5,
                        bar_phys: [0; 6],
                        irq,
                    };

                    // Вычисляем физические адреса всех BAR
                    let mut device = device;
                    for i in 0..6u8 {
                        let raw = pci_read_u32(bus, dev, func, 0x10 + i * 4);
                        if raw == 0 || raw == 0xFFFFFFFF {
                            continue;
                        }
                        if raw & 1 == 0 {
                            // Memory BAR
                            let bar_type = (raw >> 1) & 0x3;
                            if bar_type == 2 && i < 5 {
                                // 64-bit: объединяем BAR[n] + BAR[n+1]
                                let high = pci_read_u32(bus, dev, func, 0x10 + (i + 1) * 4);
                                device.bar_phys[i as usize] = ((high as u64) << 32) | ((raw as u64) & 0xFFFFFFF0);
                            } else {
                                device.bar_phys[i as usize] = raw as u64 & 0xFFFFFFF0;
                            }
                        } else {
                            // I/O BAR
                            device.bar_phys[i as usize] = (raw & !0x3) as u64;
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
    }

    println!("[PCI] Found {} devices", count);

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
