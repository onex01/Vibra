// USB xHCI — базовый драйвер контроллеров USB (xHCI/eHCI).
//
// Сканирует PCI на наличие USB контроллеров (class 0x0C, subclass 0x03).
// Читает базовые регистры MMIO: CAPLENGTH, HCIVERSION, HCSPARAMS1.
// Пока только детекция — полная инициализация xHCI требует数百 строк кода.

use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::Ordering;

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
                println!("[USB] [{:02X}:{:02X}.{}] BAR0 is zero, пропуск",
                    pci_dev.bus, pci_dev.dev, pci_dev.func);
                continue;
            }

            // Преобразуем физический адрес через HHDM
            let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
            let mmio_virt = hhdm + bar0_phys;

            // Читаем Capability Registers
            unsafe {
                let cap_length = core::ptr::read_volatile(mmio_virt as *const u8) as u64;
                let hci_version = core::ptr::read_volatile((mmio_virt + 0x02) as *const u16);
                let hcsparams1 = core::ptr::read_volatile((mmio_virt + 0x04) as *const u32);
                let port_count = (hcsparams1 & 0xFF) as u8;

                let version_major = (hci_version >> 8) as u8;
                let version_minor = (hci_version & 0xFF) as u8;

                println!("[USB] [{:02X}:{:02X}.{}] {:04X}:{:04X} xHCI v{}.{} ({} ports, CAPLEN={})",
                    pci_dev.bus, pci_dev.dev, pci_dev.func,
                    pci_dev.vendor_id, pci_dev.device_id,
                    version_major, version_minor,
                    port_count, cap_length);

                controllers.push(UsbController {
                    vendor: pci_dev.vendor_id,
                    device: pci_dev.device_id,
                    mmio_base: bar0_phys,
                    port_count,
                    version: hci_version,
                });
            }
        } else {
            println!("[USB] [{:02X}:{:02X}.{}] I/O BAR, пропуск (требуется memory BAR)",
                pci_dev.bus, pci_dev.dev, pci_dev.func);
        }
    }

    println!("[USB] {} controller(s) initialized", controllers.len());
}
