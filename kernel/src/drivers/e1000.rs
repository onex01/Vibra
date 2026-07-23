// e1000 — драйвер сети Intel Ethernet.
//
// e1000 — сетевой контроллер Intel (PCI vendor 0x8086, device 0x100E).
// Использует MMIO регистры через BAR0.
// Поддерживает передачу и приём Ethernet кадров.
//
// Регистры (смещения от BAR0):
//   CTRL   (0x000) — Device Control
//   STATUS (0x008) — Device Status
//   RCTL   (0x100) — Receive Control
//   TCTL   (0x400) — Transmit Control
//   RDBAL  (0x2800) — RX Descriptor Base Low
//   RDBAH  (0x2804) — RX Descriptor Base High
//   RDLEN  (0x2808) — RX Descriptor Length
//   RDH    (0x2810) — RX Descriptor Head
//   RDT    (0x2818) — RX Descriptor Tail
//   TDBAL  (0x3800) — TX Descriptor Base Low
//   TDBAH  (0x3804) — TX Descriptor Base High
//   TDLEN  (0x3808) — TX Descriptor Length
//   TDH    (0x3810) — TX Descriptor Head
//   TDT    (0x3818) — TX Descriptor Tail
//   RAL    (0x5400) — Receive Address Low (MAC)
//   RAH    (0x5404) — Receive Address High

use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::{AtomicU64, Ordering};

// e1000 регистры
const E1000_CTRL: u64 = 0x000;
const E1000_STATUS: u64 = 0x008;
const E1000_RCTL: u64 = 0x100;
const E1000_TCTL: u64 = 0x400;
const E1000_RDBAL: u64 = 0x2800;
const E1000_RDBAH: u64 = 0x2804;
const E1000_RDLEN: u64 = 0x2808;
const E1000_RDH: u64 = 0x2810;
const E1000_RDT: u64 = 0x2818;
const E1000_TDBAL: u64 = 0x3800;
const E1000_TDBAH: u64 = 0x3804;
const E1000_TDLEN: u64 = 0x3808;
const E1000_TDH: u64 = 0x3810;
const E1000_TDT: u64 = 0x3818;
const E1000_RAL: u64 = 0x5400;
const E1000_RAH: u64 = 0x5404;

// CTRL биты
const CTRL_RESET: u32 = 1 << 26;     // Reset
const CTRL_SLU: u32 = 1 << 6;        // Set Link Up

// RCTL биты
const RCTL_EN: u32 = 1 << 1;         // Receiver Enable
const RCTL_BAM: u32 = 1 << 15;       // Broadcast Accept Mode
const RCTL_BSIZE_2048: u32 = 0;      // Buffer Size: 2048 bytes
const RCTL_BSEX: u32 = 1 << 25;      // Buffer Size Extension

// TCTL биты
const TCTL_EN: u32 = 1 << 1;         // Transmitter Enable
const TCTL_PSP: u32 = 1 << 3;        // Pad Short Packets
const TCTL_CT: u32 = 0x0F << 4;      // Collision Threshold
const TCTL_COLD: u32 = 0x40 << 12;   // Collision Distance

// RX/TX дескрипторы
const RX_DESC_COUNT: usize = 32;
const TX_DESC_COUNT: usize = 8;
const RX_BUFFER_SIZE: usize = 2048;

// RX Descriptor status bits
const RXD_STAT_DD: u8 = 1;           // Descriptor Done

#[repr(C, packed)]
struct RxDescriptor {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C, packed)]
struct TxDescriptor {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

// MMIO базовый адрес
static E1000_BASE: AtomicU64 = AtomicU64::new(0);

#[inline]
unsafe fn e1000_read32(offset: u64) -> u32 {
    let base = E1000_BASE.load(Ordering::Relaxed);
    core::ptr::read_volatile((base + offset) as *const u32)
}

#[inline]
unsafe fn e1000_write32(offset: u64, val: u32) {
    let base = E1000_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

fn virt_to_phys(virt: u64) -> u64 {
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    virt - hhdm
}

/// e1000 контроллер
pub struct E1000Controller {
    mmio_base: u64,
    mac: [u8; 6],
    // RX
    rx_descs: *mut RxDescriptor,
    rx_descs_phys: u64,
    rx_buffers: Vec<*mut u8>,
    rx_buffers_phys: Vec<u64>,
    rx_head: u16,
    rx_tail: u16,
    // TX
    tx_descs: *mut TxDescriptor,
    tx_descs_phys: u64,
    tx_buffers: Vec<*mut u8>,
    tx_buffers_phys: Vec<u64>,
    tx_head: u16,
    tx_tail: u16,
}

unsafe impl Send for E1000Controller {}
unsafe impl Sync for E1000Controller {}

impl E1000Controller {
    /// Сброс контроллера
    unsafe fn reset(&self) {
        e1000_write32(E1000_CTRL, CTRL_RESET);

        // Ждём завершения сброса
        let mut timeout = 10_000_000u32;
        while e1000_read32(E1000_CTRL) & CTRL_RESET != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 {
            println!("[e1000] Предупреждение: тайм-аут сброса");
        }
    }

    /// Прочитать MAC адрес
    unsafe fn read_mac(&mut self) {
        let ral = e1000_read32(E1000_RAL);
        let rah = e1000_read32(E1000_RAH);

        self.mac[0] = ral as u8;
        self.mac[1] = (ral >> 8) as u8;
        self.mac[2] = (ral >> 16) as u8;
        self.mac[3] = (ral >> 24) as u8;
        self.mac[4] = rah as u8;
        self.mac[5] = (rah >> 8) as u8;
    }

    /// Настроить RX дескрипторы
    unsafe fn setup_rx(&mut self) {
        // Выделяем дескрипторы
        let desc_size = core::mem::size_of::<RxDescriptor>() * RX_DESC_COUNT;
        let desc_layout = core::alloc::Layout::from_size_align(desc_size, 16).unwrap();
        self.rx_descs = alloc::alloc::alloc_zeroed(desc_layout) as *mut RxDescriptor;
        self.rx_descs_phys = virt_to_phys(self.rx_descs as u64);

        // Выделяем RX буферы
        for _ in 0..RX_DESC_COUNT {
            let buf_layout = core::alloc::Layout::from_size_align(RX_BUFFER_SIZE, 16).unwrap();
            let buf = alloc::alloc::alloc_zeroed(buf_layout);
            let buf_phys = virt_to_phys(buf as u64);
            self.rx_buffers.push(buf);
            self.rx_buffers_phys.push(buf_phys);
        }

        // Заполняем RX дескрипторы
        for i in 0..RX_DESC_COUNT {
            (*self.rx_descs.add(i)).addr = self.rx_buffers_phys[i];
        }

        self.rx_head = 0;
        self.rx_tail = (RX_DESC_COUNT - 1) as u16;

        // Устанавливаем адреса дескрипторов
        e1000_write32(E1000_RDBAL, self.rx_descs_phys as u32);
        e1000_write32(E1000_RDBAH, (self.rx_descs_phys >> 32) as u32);
        e1000_write32(E1000_RDLEN, desc_size as u32);
        e1000_write32(E1000_RDH, 0);
        e1000_write32(E1000_RDT, (RX_DESC_COUNT - 1) as u32);
    }

    /// Настроить TX дескрипторы
    unsafe fn setup_tx(&mut self) {
        let desc_size = core::mem::size_of::<TxDescriptor>() * TX_DESC_COUNT;
        let desc_layout = core::alloc::Layout::from_size_align(desc_size, 16).unwrap();
        self.tx_descs = alloc::alloc::alloc_zeroed(desc_layout) as *mut TxDescriptor;
        self.tx_descs_phys = virt_to_phys(self.tx_descs as u64);

        // Выделяем TX буферы
        for _ in 0..TX_DESC_COUNT {
            let buf_layout = core::alloc::Layout::from_size_align(RX_BUFFER_SIZE, 16).unwrap();
            let buf = alloc::alloc::alloc_zeroed(buf_layout);
            let buf_phys = virt_to_phys(buf as u64);
            self.tx_buffers.push(buf);
            self.tx_buffers_phys.push(buf_phys);
        }

        self.tx_head = 0;
        self.tx_tail = 0;

        // Устанавливаем адреса дескрипторов
        e1000_write32(E1000_TDBAL, self.tx_descs_phys as u32);
        e1000_write32(E1000_TDBAH, (self.tx_descs_phys >> 32) as u32);
        e1000_write32(E1000_TDLEN, desc_size as u32);
        e1000_write32(E1000_TDH, 0);
        e1000_write32(E1000_TDT, 0);
    }

    /// Включить приёмник и передатчик
    unsafe fn enable(&self) {
        // Включаем RCTL
        e1000_write32(E1000_RCTL, RCTL_EN | RCTL_BAM | RCTL_BSIZE_2048);

        // Включаем TCTL
        e1000_write32(E1000_TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD);

        // Set Link Up
        let ctrl = e1000_read32(E1000_CTRL);
        e1000_write32(E1000_CTRL, ctrl | CTRL_SLU);
    }
}

/// Глобальный e1000 контроллер
static E1000_CONTROLLER: Mutex<Option<E1000Controller>> = Mutex::new(None);

/// Инициализация e1000 драйвера
pub fn init() {
    println!("[e1000] Поиск Intel e1000 контроллера...");

    // Ищем e1000 через PCI (vendor 0x8086, device 0x100E)
    let e1000_device = match super::pci::find_device_by_id(0x8086, 0x100E) {
        Some(d) => d,
        None => {
            println!("[e1000] Intel e1000 контроллер не найден");
            return;
        }
    };

    println!("[e1000] Найден контроллер PCI [{:02X}:{:02X}.{}]",
        e1000_device.bus, e1000_device.dev, e1000_device.func);

    // Включаем Bus Master и Memory Space
    unsafe {
        super::pci::enable_bus_master(e1000_device.bus, e1000_device.dev, e1000_device.func);
    }

    // BAR0 — MMIO регистры
    let bar0 = e1000_device.bar0 & 0xFFFFFFF0;
    if bar0 == 0 {
        println!("[e1000] ОШИБКА: BAR0 = 0 — контроллер не сконфигурирован");
        return;
    }

    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let mmio_base = hhdm + bar0 as u64;
    E1000_BASE.store(mmio_base, Ordering::SeqCst);

    println!("[e1000] BAR0: физ={:#x} вирт={:#x}", bar0, mmio_base);

    let mut controller = E1000Controller {
        mmio_base,
        mac: [0u8; 6],
        rx_descs: core::ptr::null_mut(),
        rx_descs_phys: 0,
        rx_buffers: Vec::new(),
        rx_buffers_phys: Vec::new(),
        rx_head: 0,
        rx_tail: 0,
        tx_descs: core::ptr::null_mut(),
        tx_descs_phys: 0,
        tx_buffers: Vec::new(),
        tx_buffers_phys: Vec::new(),
        tx_head: 0,
        tx_tail: 0,
    };

    unsafe {
        // Сброс контроллера
        controller.reset();
        println!("[e1000] Контроллер сброшен");

        // Читаем MAC
        controller.read_mac();
        println!("[e1000] MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            controller.mac[0], controller.mac[1], controller.mac[2],
            controller.mac[3], controller.mac[4], controller.mac[5]);

        // Настраиваем RX/TX
        controller.setup_rx();
        controller.setup_tx();
        println!("[e1000] RX/TX дескрипторы настроены");

        // Включаем приёмник и передатчик
        controller.enable();
        println!("[e1000] RCTL/TCTL включены");
    }

    println!("[e1000] Драйвер e1000 инициализирован успешно");

    *E1000_CONTROLLER.lock() = Some(controller);
}

/// Получить MAC адрес
pub fn get_mac() -> Option<[u8; 6]> {
    let ctrl = E1000_CONTROLLER.lock();
    ctrl.as_ref().map(|c| c.mac)
}

/// Отправить Ethernet кадр
pub fn send_packet(data: &[u8]) -> bool {
    let mut ctrl_guard = E1000_CONTROLLER.lock();
    let ctrl = match ctrl_guard.as_mut() {
        Some(c) => c,
        None => return false,
    };

    unsafe {
        let slot = ctrl.tx_tail as usize;
        let data_len = data.len().min(RX_BUFFER_SIZE);

        // Копируем данные в TX буфер
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            ctrl.tx_buffers[slot],
            data_len,
        );

        // Заполняем TX дескриптор
        (*ctrl.tx_descs.add(slot)).addr = ctrl.tx_buffers_phys[slot];
        (*ctrl.tx_descs.add(slot)).length = data_len as u16;
        (*ctrl.tx_descs.add(slot)).cmd = 0x0B; // RS + IFCS + EOP
        (*ctrl.tx_descs.add(slot)).status = 0;

        // Обновляем Tail
        ctrl.tx_tail = (ctrl.tx_tail + 1) % TX_DESC_COUNT as u16;
        e1000_write32(E1000_TDT, ctrl.tx_tail as u32);
    }

    true
}

/// Принять Ethernet кадр (максимальный размер 1518 байт)
pub fn recv_packet(buf: &mut [u8]) -> Option<usize> {
    let mut ctrl_guard = E1000_CONTROLLER.lock();
    let ctrl = match ctrl_guard.as_mut() {
        Some(c) => c,
        None => return None,
    };

    unsafe {
        let slot = ctrl.rx_head as usize;
        let desc = &*ctrl.rx_descs.add(slot);

        if desc.status & RXD_STAT_DD == 0 {
            return None; // Нет нового пакета
        }

        let len = desc.length as usize;
        let copy_len = len.min(buf.len());

        // Копируем данные из RX буфера
        core::ptr::copy_nonoverlapping(ctrl.rx_buffers[slot], buf.as_mut_ptr(), copy_len);

        // Сбрасываем дескриптор
        (*ctrl.rx_descs.add(slot)).status = 0;
        (*ctrl.rx_descs.add(slot)).length = 0;

        // Обновляем Head
        ctrl.rx_head = (ctrl.rx_head + 1) % RX_DESC_COUNT as u16;
        e1000_write32(E1000_RDH, ctrl.rx_head as u32);

        Some(copy_len)
    }
}
