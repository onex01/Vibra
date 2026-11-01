// USB xHCI — базовый драйвер контроллеров USB (xHCI/eHCI).
//
// Сканирует PCI на наличие USB контроллеров (class 0x0C, subclass 0x03).
// Читает базовые регистры MMIO: CAPLENGTH, HCIVERSION, HCSPARAMS1.
// Пока только детекция — полная инициализация xHCI требует数百 строк кода.

use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::Ordering;

pub mod hid;
pub mod storage;
pub mod uhci;
pub mod xhci;

/// Информация об обнаруженном USB контроллере
#[derive(Debug, Clone)]
pub struct UsbController {
    pub vendor: u16,
    pub device: u16,
    pub mmio_base: u64,
    pub port_count: u8,
    pub version: u16,
}

static USB_CONTROLLERS: Mutex<Vec<UsbController>> = Mutex::new(Vec::new());

/// Получить список найденных USB контроллеров
pub fn get_controllers() -> Vec<UsbController> {
    USB_CONTROLLERS.lock().clone()
}

/// USB Device Descriptor (parsed minimal fields)
#[derive(Debug, Clone)]
pub struct UsbDeviceDescriptor {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_subclass: u8,
    pub b_device_protocol: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub b_num_configurations: u8,
}

/// Representation of a discovered USB device inside the kernel
#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub vendor: u16,
    pub product: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
}

static USB_DEVICES: Mutex<Vec<UsbDevice>> = Mutex::new(Vec::new());

/// Parse a device descriptor buffer (18 bytes) into `UsbDeviceDescriptor`.
pub fn parse_device_descriptor(buf: &[u8]) -> Option<UsbDeviceDescriptor> {
    if buf.len() < 18 { return None; }
    if buf[1] != 0x01 { return None; } // bDescriptorType == DEVICE
    let bcd_usb = u16::from_le_bytes([buf[2], buf[3]]);
    let b_device_class = buf[4];
    let b_device_subclass = buf[5];
    let b_device_protocol = buf[6];
    let id_vendor = u16::from_le_bytes([buf[8], buf[9]]);
    let id_product = u16::from_le_bytes([buf[10], buf[11]]);
    let b_num_configurations = buf[17];
    Some(UsbDeviceDescriptor {
        bcd_usb,
        b_device_class,
        b_device_subclass,
        b_device_protocol,
        id_vendor,
        id_product,
        b_num_configurations,
    })
}

/// Register a USB device in kernel device registry based on descriptor.
pub fn register_device_from_descriptor(desc: &UsbDeviceDescriptor) {
    // Map USB class to kernel DeviceType
    use crate::kernel::device::{register as register_dev, DeviceType};

    let dev = UsbDevice {
        vendor: desc.id_vendor,
        product: desc.id_product,
        device_class: desc.b_device_class,
        device_subclass: desc.b_device_subclass,
        device_protocol: desc.b_device_protocol,
    };

    // Decide device type
    let dtype = match desc.b_device_class {
        0x03 => { // HID
            // protocol 1 = keyboard, 2 = mouse (when device-level class used)
            match desc.b_device_protocol {
                0x01 => DeviceType::Keyboard,
                0x02 => DeviceType::Mouse,
                _ => DeviceType::Unknown,
            }
        }
        0x08 => DeviceType::Disk, // Mass Storage
        _ => DeviceType::Unknown,
    };

    let name = alloc::format!("usb:{:04x}:{:04x}", desc.id_vendor, desc.id_product);
    let id = register_dev(&name, dtype);
    println!("[USB] registered device id={} name={} class={:#x}", id, name, desc.b_device_class);

    USB_DEVICES.lock().push(dev);
}

pub fn get_devices() -> Vec<UsbDevice> {
    USB_DEVICES.lock().clone()
}

/// Инициализация подсистемы USB — поиск контроллеров через PCI.
///
/// Тип контроллера определяется по `prog_if` (PCI class 0x0C / subclass 0x03):
///   0x00 — UHCI (USB 1.1, Intel/VIA): low/full-speed устройства, включая
///          клавиатуры на USB2-портах чипсетов Intel и AMD SB;
///   0x10 — OHCI (USB 1.1, NEC/AMD/VIA) — не поддерживается;
///   0x20 — EHCI (USB 2.0) — не поддерживается (см. docs/usb_and_boot.md);
///   0x30 — xHCI (USB 3.x) — основной драйвер.
///
/// Раньше тип не проверялся: EHCI-контроллеры отдавались в xHCI-код и
/// падали с «invalid topology (slots=2, ports=0)», а клавиатуры на них
/// оставались невидимыми.
pub fn init() {
    let devices = super::pci::find_all_devices_by_class(0x0C, 0x03);

    if devices.is_empty() {
        println!("[USB] No USB controllers found");
        return;
    }

    println!("[USB] Found {} USB controller(s)", devices.len());

    let mut found: Vec<UsbController> = Vec::new();
    let mut uhci_count = 0usize;

    for pci_dev in &devices {
        let kind = match pci_dev.prog_if {
            0x00 => "UHCI",
            0x10 => "OHCI",
            0x20 => "EHCI",
            0x30 => "xHCI",
            other => {
                println!(
                    "[USB] [{:02X}:{:02X}.{}] unknown prog_if {:#04x}, skipping",
                    pci_dev.bus, pci_dev.dev, pci_dev.func, other
                );
                continue;
            }
        };

        unsafe {
            super::pci::enable_bus_master(pci_dev.bus, pci_dev.dev, pci_dev.func);
        }

        match pci_dev.prog_if {
            0x30 => {
                if pci_dev.is_io_bar(0) {
                    println!("[USB] xHCI with I/O BAR0, skipping");
                    continue;
                }
                let bar0_phys = pci_dev.bar0_phys();
                if bar0_phys == 0 {
                    println!("[USB] [{:02X}:{:02X}.{}] BAR0 is zero, skipping",
                        pci_dev.bus, pci_dev.dev, pci_dev.func);
                    continue;
                }
                let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
                let mmio_virt = hhdm + bar0_phys;

                let (hci_version, port_count) = unsafe {
                    let cap0 = core::ptr::read_volatile(mmio_virt as *const u32);
                    let hcsparams1 = core::ptr::read_volatile((mmio_virt + 0x04) as *const u32);
                    let hci_version = (cap0 >> 16) as u16;
                    let port_count = ((hcsparams1 >> 24) & 0xFF) as u8;
                    println!(
                        "[USB] [{:02X}:{:02X}.{}] {:04X}:{:04X} {} v{}.{} ({} ports, CAPLEN={})",
                        pci_dev.bus, pci_dev.dev, pci_dev.func,
                        pci_dev.vendor_id, pci_dev.device_id,
                        kind, hci_version >> 8, hci_version & 0xFF,
                        port_count, cap0 & 0xFF
                    );
                    (hci_version, port_count)
                };

                if crate::drivers::usb::xhci::init_controller(bar0_phys, mmio_virt) {
                    found.push(UsbController {
                        vendor: pci_dev.vendor_id,
                        device: pci_dev.device_id,
                        mmio_base: bar0_phys,
                        port_count,
                        version: hci_version,
                    });
                }
            }
            0x00 => {
                // UHCI: I/O BAR4, 32-байтное выравнивание.
                let io_base = (pci_dev.bar4 & !0x1F) as u16;
                println!(
                    "[USB] [{:02X}:{:02X}.{}] {:04X}:{:04X} UHCI (I/O {:#06x})",
                    pci_dev.bus, pci_dev.dev, pci_dev.func,
                    pci_dev.vendor_id, pci_dev.device_id, io_base
                );
                if crate::drivers::usb::uhci::init(io_base) {
                    uhci_count += 1;
                }
            }
            other => {
                println!(
                    "[USB] [{:02X}:{:02X}.{}] {:04X}:{:04X} {} (prog_if {:#04x}) is not supported yet",
                    pci_dev.bus, pci_dev.dev, pci_dev.func,
                    pci_dev.vendor_id, pci_dev.device_id, kind, other
                );
            }
        }
    }

    let initialized = found.len() + uhci_count;
    USB_CONTROLLERS.lock().extend(found);
    println!(
        "[USB] {} controller(s) initialized ({} xHCI, {} UHCI)",
        initialized,
        USB_CONTROLLERS.lock().len(),
        uhci_count
    );

    crate::drivers::usb::hid::init();
}
