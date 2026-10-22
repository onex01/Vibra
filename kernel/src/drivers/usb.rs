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

/// Инициализация подсистемы USB — поиск контроллеров через PCI
pub fn init() {
    // USB контроллеры: class = 0x0C (Serial Bus), subclass = 0x03 (USB)
    let devices = super::pci::find_all_devices_by_class(0x0C, 0x03);

    if devices.is_empty() {
        println!("[USB] No USB controllers found");
        return;
    }

    println!("[USB] Found {} USB controller(s)", devices.len());

    let mut controllers = USB_CONTROLLERS.lock();

    for pci_dev in &devices {
        // Включаем Bus Master и Memory Space
        unsafe {
            super::pci::enable_bus_master(pci_dev.bus, pci_dev.dev, pci_dev.func);
        }

        // BAR0 — MMIO базовый адрес (если memory BAR)
        if !pci_dev.is_io_bar(0) {
            let bar0_phys = pci_dev.bar0_phys();
            if bar0_phys == 0 {
                println!("[USB] [{:02X}:{:02X}.{}] BAR0 is zero, skipping",
                    pci_dev.bus, pci_dev.dev, pci_dev.func);
                continue;
            }

            // Преобразуем физический адрес через HHDM
            let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
            let mmio_virt = hhdm + bar0_phys;

            // Capability registers: версия и число портов (HCSPARAMS1 bits 24-31).
            let (hci_version, port_count) = unsafe {
                let cap0 = core::ptr::read_volatile(mmio_virt as *const u32);
                let cap_length = (cap0 & 0xFF) as u64;
                let hci_version = (cap0 >> 16) as u16;
                let hcsparams1 = core::ptr::read_volatile((mmio_virt + 0x04) as *const u32);
                let port_count = ((hcsparams1 >> 24) & 0xFF) as u8;

                println!(
                    "[USB] [{:02X}:{:02X}.{}] {:04X}:{:04X} xHCI v{}.{} ({} ports, CAPLEN={})",
                    pci_dev.bus, pci_dev.dev, pci_dev.func,
                    pci_dev.vendor_id, pci_dev.device_id,
                    hci_version >> 8, hci_version & 0xFF,
                    port_count, cap_length
                );
                (hci_version, port_count)
            };

            // Полный lifecycle: BIOS handoff, reset, DCBAA, rings, enumeration.
            if crate::drivers::usb::xhci::init_controller(bar0_phys, mmio_virt) {
                controllers.push(UsbController {
                    vendor: pci_dev.vendor_id,
                    device: pci_dev.device_id,
                    mmio_base: bar0_phys,
                    port_count,
                    version: hci_version,
                });
            }
        } else {
            println!("[USB] [{:02X}:{:02X}.{}] I/O BAR, skipping (memory BAR required)",
                pci_dev.bus, pci_dev.dev, pci_dev.func);
        }
    }

    println!("[USB] {} controller(s) initialized", controllers.len());

    // HID: отчёты приходят из xhci::poll_keyboard().
    crate::drivers::usb::hid::init();
}
