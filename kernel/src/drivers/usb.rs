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

            // Читаем Capability Registers и пробуем минимальный reset xHCI
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

                // Операционные регистры начинаются по смещению cap_length
                let op_base = mmio_virt + cap_length as u64;

                // USBCMD offset 0x00 (u32). HCRST = bit 1.
                let usbcmd_ptr = op_base as *mut u32;
                let usbsts_ptr = (op_base + 0x04) as *mut u32;

                // Попробуем сбросить контроллер (HCRST)
                let mut cmd = core::ptr::read_volatile(usbcmd_ptr);
                core::ptr::write_volatile(usbcmd_ptr, cmd | 0x02);
                // Ждём до 10000 итераций
                let mut waited = 0;
                while (core::ptr::read_volatile(usbcmd_ptr) & 0x02) != 0 {
                    waited += 1;
                    if waited > 10000 {
                        println!("[USB] xHCI reset timeout (waited {})", waited);
                        break;
                    }
                }

                let final_cmd = core::ptr::read_volatile(usbcmd_ptr);
                let sts = core::ptr::read_volatile(usbsts_ptr);
                println!("[USB] xHCI reset complete: USBCMD={:#x} USBSTS={:#x}", final_cmd, sts);
                // Попробуем прочитать PORTSC регистры (на offset 0x400 от op_base)
                let portsc_base = op_base + 0x400;
                for p in 0..port_count {
                    let port_off = portsc_base + (p as u64) * 0x10;
                    let portsc = core::ptr::read_volatile(port_off as *const u32);
                    // bit 0 = Current Connect Status (CCS)
                    let ccs = portsc & 0x1 != 0;
                    println!("[USB] port {}: PORTSC={:#x} connected={}", p+1, portsc, ccs);
                }

                controllers.push(UsbController {
                    vendor: pci_dev.vendor_id,
                    device: pci_dev.device_id,
                    mmio_base: bar0_phys,
                    port_count,
                    version: hci_version,
                });
                // Initialize xHCI admin queues for this controller
                crate::drivers::usb::xhci::init_controller(bar0_phys, mmio_virt);
                // Allocate and set up Event Ring / ERST structures (logging only for now)
                crate::drivers::usb::xhci::init_event_ring(bar0_phys, mmio_virt);
            }
        } else {
            println!("[USB] [{:02X}:{:02X}.{}] I/O BAR, пропуск (требуется memory BAR)",
                pci_dev.bus, pci_dev.dev, pci_dev.func);
        }
    }

    println!("[USB] {} controller(s) initialized", controllers.len());

    // Инициализация подсистемы HID (минимальная) — регистрируем тестовую строку
    // и готовим пространство для дальнейшей реализации HID.
    crate::drivers::usb::hid::init();
}
