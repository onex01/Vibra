// RTL8139 — драйвер сети Realtek Ethernet.
//
// RTL8139 — сетевой контроллер Realtek (PCI vendor 0x10EC, device 0x8139).
// Использует I/O порты через BAR0.
// Простой и надёжный контроллер, широко распространён.
//
// Регистры (смещения от I/O base):
//   MAC0     (0x00) — MAC адрес (6 байт)
//   MAR0     (0x08) — Multicast Address Register
//   RBSTART  (0x30) — RX Buffer Starting Address
//   CMD      (0x37) — Command Register
//   CAPR     (0x38) — Current Address of Packet Read
//   CBR      (0x3A) — Current Buffer Read
//   ISR      (0x3E) — Interrupt Status Register
//   RCR      (0x44) — Receive Configuration Register
//   TCR      (0x40) — Transmit Configuration Register
//   TXADDR0  (0x20) — Transmit Buffer Address (4 порта)
//   TXSTATUS0 (0x10) — Transmit Status of Descriptor 0
//   TSAD0    (0x10) — Transmit Status of Address

use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::{AtomicBool, Ordering};

// RTL8139 регистры
const RTL_MAC0: u16 = 0x00;
const RTL_CMD: u16 = 0x37;
const RTL_RBSTART: u16 = 0x30;
const RTL_CAPR: u16 = 0x38;
const RTL_CBR: u16 = 0x3A;
const RTL_ISR: u16 = 0x3E;
const RTL_RCR: u16 = 0x44;
const RTL_TCR: u16 = 0x40;
const RTL_TXADDR0: u16 = 0x20;
const RTL_TXSTATUS0: u16 = 0x10;

// CMD биты
const CMD_RESET: u8 = 0x10;     // Software Reset
const CMD_RX_EN: u8 = 0x08;     // Receiver Enable
const CMD_TX_EN: u8 = 0x04;     // Transmitter Enable
const CMD_RXBUF_EMPTY: u8 = 0x01; // Rx Buffer Empty

// RCR биты
const RCR_AAM: u8 = 0x01;      // Accept All Multicast
const RCR_APM: u8 = 0x02;      // Accept Physical Match
const RCR_AB: u8 = 0x04;       // Accept Broadcast
const RCR_AR: u8 = 0x08;       // Accept Runt (< 64 bytes)
const RCR_AER: u8 = 0x10;      // Accept Error Packets
const RCR_WRAP: u8 = 0x80;     // Wrap (не переполнять RX буфер)
const RCR_CONFIG: u8 = RCR_AAM | RCR_APM | RCR_AB | RCR_WRAP;

// TCR биты
const TCR_IFG: u32 = 11 << 24; // Interframe Gap

// ISR биты
const ISR_ROK: u16 = 0x01;     // Receive OK
const ISR_TOK: u16 = 0x10;     // Transmit OK
const ISR_RER: u16 = 0x02;     // Receive Error
const ISR_TER: u16 = 0x20;     // Transmit Error

// RX буфер: 16K + 1518 + 16 (для wraparound)
const RX_BUFFER_SIZE: usize = 16384 + 1518 + 16;
// RX дескрипторы — RTL8139 не использует отдельные дескрипторы, просто кольцевой буфер

// TX буферы: 4 порта по 1536 байт
const TX_BUFFER_SIZE: usize = 1536;
const TX_PORT_COUNT: usize = 4;

#[inline]
unsafe fn rtl_inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline]
unsafe fn rtl_outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn rtl_inw(port: u16) -> u16 {
    let val: u16;
    core::arch::asm!("in ax, dx", out("ax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline]
unsafe fn rtl_outw(port: u16, val: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
}

#[inline]
unsafe fn rtl_inl(port: u16) -> u32 {
    let val: u32;
    core::arch::asm!("in eax, dx", out("eax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline]
unsafe fn rtl_outl(port: u16, val: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
}

fn virt_to_phys(virt: u64) -> u64 {
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    virt - hhdm
}

/// RTL8139 контроллер
pub struct Rtl8139Controller {
    io_base: u16,
    mac: [u8; 6],
    // RX
    rx_buffer: *mut u8,
    rx_buffer_phys: u64,
    rx_pos: usize,      // Текущая позиция чтения
    // TX
    tx_buffers: [*mut u8; TX_PORT_COUNT],
    tx_buffers_phys: [u64; TX_PORT_COUNT],
    tx_cur_port: usize, // Текущий TX порт (0-3)
}

unsafe impl Send for Rtl8139Controller {}
unsafe impl Sync for Rtl8139Controller {}

impl Rtl8139Controller {
    /// Программный сброс
    unsafe fn reset(&self) {
        rtl_outb(self.io_base + RTL_CMD, CMD_RESET);

        // Ждём завершения сброса
        let mut timeout = 10_000_000u32;
        while rtl_inb(self.io_base + RTL_CMD) & CMD_RESET != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 {
            println!("[RTL8139] Предупреждение: тайм-аут сброса");
        }
    }

    /// Прочитать MAC адрес из регистра
    unsafe fn read_mac(&mut self) {
        for i in 0..6 {
            self.mac[i] = rtl_inb(self.io_base + RTL_MAC0 + i as u16);
        }
    }

    /// Настроить RX буфер
    unsafe fn setup_rx(&mut self) {
        let buf_layout = core::alloc::Layout::from_size_align(RX_BUFFER_SIZE, 256).unwrap();
        self.rx_buffer = alloc::alloc::alloc_zeroed(buf_layout);
        if self.rx_buffer.is_null() {
            println!("[RTL8139] ОШИБКА: Не удалось выделить RX буфер");
            return;
        }
        self.rx_buffer_phys = virt_to_phys(self.rx_buffer as u64);
        self.rx_pos = 0;

        // Устанавливаем RBSTART
        rtl_outl(self.io_base + RTL_RBSTART, self.rx_buffer_phys as u32);
    }

    /// Настроить TX буферы
    unsafe fn setup_tx(&mut self) {
        for i in 0..TX_PORT_COUNT {
            let buf_layout = core::alloc::Layout::from_size_align(TX_BUFFER_SIZE, 256).unwrap();
            self.tx_buffers[i] = alloc::alloc::alloc_zeroed(buf_layout);
            if self.tx_buffers[i].is_null() {
                println!("[RTL8139] ОШИБКА: Не удалось выделить TX буфер {}", i);
                return;
            }
            self.tx_buffers_phys[i] = virt_to_phys(self.tx_buffers[i] as u64);

            // Устанавливаем TXADDR0 + i*4
            rtl_outl(self.io_base + RTL_TXADDR0 + (i as u16) * 4, self.tx_buffers_phys[i] as u32);
        }
        self.tx_cur_port = 0;
    }

    /// Включить приёмник и передатчик
    unsafe fn enable(&self) {
        // RCR — конфигурация приёмника
        rtl_outb(self.io_base + RTL_RCR, RCR_CONFIG);

        // TCR — конфигурация передатчика
        rtl_outl(self.io_base + RTL_TCR, TCR_IFG);

        // Включаем RX и TX
        rtl_outb(self.io_base + RTL_CMD, CMD_RX_EN | CMD_TX_EN);
    }
}

/// Глобальный RTL8139 контроллер
static RTL8139_CONTROLLER: Mutex<Option<Rtl8139Controller>> = Mutex::new(None);
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Инициализация RTL8139 драйвера
pub fn init() {
    if INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    println!("[RTL8139] Поиск Realtek RTL8139 контроллера...");

    // Ищем RTL8139 через PCI (vendor 0x10EC, device 0x8139)
    let rtl_device = match super::pci::find_device_by_id(0x10EC, 0x8139) {
        Some(d) => d,
        None => {
            println!("[RTL8139] Realtek RTL8139 контроллер не найден");
            return;
        }
    };

    println!("[RTL8139] Найден контроллер PCI [{:02X}:{:02X}.{}]",
        rtl_device.bus, rtl_device.dev, rtl_device.func);

    // Включаем Bus Master и I/O Space
    unsafe {
        super::pci::enable_bus_master(rtl_device.bus, rtl_device.dev, rtl_device.func);
    }

    // BAR0 — I/O порты (RTL8139 использует I/O BAR)
    if !rtl_device.is_io_bar(0) {
        println!("[RTL8139] ОШИБКА: BAR0 не является I/O BAR");
        return;
    }

    let io_base = rtl_device.bar0 as u16 & 0xFFFC;
    if io_base == 0 {
        println!("[RTL8139] ОШИБКА: BAR0 = 0 — контроллер не сконфигурирован");
        return;
    }

    println!("[RTL8139] I/O base: {:#x}", io_base);

    let mut controller = Rtl8139Controller {
        io_base,
        mac: [0u8; 6],
        rx_buffer: core::ptr::null_mut(),
        rx_buffer_phys: 0,
        rx_pos: 0,
        tx_buffers: [core::ptr::null_mut(); TX_PORT_COUNT],
        tx_buffers_phys: [0; TX_PORT_COUNT],
        tx_cur_port: 0,
    };

    unsafe {
        // Сброс контроллера
        controller.reset();
        println!("[RTL8139] Контроллер сброшен");

        // Читаем MAC
        controller.read_mac();
        println!("[RTL8139] MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            controller.mac[0], controller.mac[1], controller.mac[2],
            controller.mac[3], controller.mac[4], controller.mac[5]);

        // Настраиваем RX/TX
        controller.setup_rx();
        controller.setup_tx();
        println!("[RTL8139] RX/TX буферы настроены");

        // Включаем приёмник и передатчик
        controller.enable();
        println!("[RTL8139] RX/TX включены");
    }

    println!("[RTL8139] Драйвер RTL8139 инициализирован успешно");

    *RTL8139_CONTROLLER.lock() = Some(controller);
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Получить MAC адрес
pub fn get_mac() -> Option<[u8; 6]> {
    let ctrl = RTL8139_CONTROLLER.lock();
    ctrl.as_ref().map(|c| c.mac)
}

/// Отправить Ethernet кадр
pub fn send_packet(data: &[u8]) -> bool {
    let mut ctrl_guard = RTL8139_CONTROLLER.lock();
    let ctrl = match ctrl_guard.as_mut() {
        Some(c) => c,
        None => return false,
    };

    unsafe {
        let port = ctrl.tx_cur_port;
        let data_len = data.len().min(TX_BUFFER_SIZE);

        // Копируем данные в TX буфер
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            ctrl.tx_buffers[port],
            data_len,
        );

        // Устанавливаем статус передачи:
        // биты [12:0] = размер, бит 13 = OWN (1 = передача завершена)
        let status = (data_len as u32) | (1 << 13);
        rtl_outl(ctrl.io_base + RTL_TXSTATUS0 + (port as u16) * 4, status);

        // Переключаемся на следующий порт
        ctrl.tx_cur_port = (ctrl.tx_cur_port + 1) % TX_PORT_COUNT;
    }

    true
}

/// Принять Ethernet кадр
pub fn recv_packet(buf: &mut [u8]) -> Option<usize> {
    let mut ctrl_guard = RTL8139_CONTROLLER.lock();
    let ctrl = match ctrl_guard.as_mut() {
        Some(c) => c,
        None => return None,
    };

    unsafe {
        // Проверяем CBR (Current Buffer Read)
        let cbr = rtl_inw(ctrl.io_base + RTL_CBR) as usize;

        if ctrl.rx_pos == cbr {
            return None; // Нет новых пакетов
        }

        // Читаем заголовок пакета: 2 байта статус, 2 байта длина
        let hdr_ptr = (ctrl.rx_buffer.add(ctrl.rx_pos)) as *const u16;
        let pkt_status = core::ptr::read_volatile(hdr_ptr);
        let pkt_len = core::ptr::read_volatile(hdr_ptr.add(1)) as usize;

        // Проверяем статус — пакет принят без ошибок
        if pkt_status & 0x01 == 0 {
            // Ошибка приёма, пропускаем пакет
            ctrl.rx_pos = (ctrl.rx_pos + 4 + pkt_len + 3) & !3; // Выравнивание по 4 байта
            if ctrl.rx_pos >= RX_BUFFER_SIZE {
                ctrl.rx_pos = 0;
                rtl_outb(ctrl.io_base + RTL_CAPR, (RX_BUFFER_SIZE - 16) as u8);
            }
            return None;
        }

        // Копируем данные (без заголовка 4 байта)
        let data_offset = ctrl.rx_pos + 4;
        let copy_len = (pkt_len - 4).min(buf.len()); // -4 убираем CRC

        core::ptr::copy_nonoverlapping(
            ctrl.rx_buffer.add(data_offset),
            buf.as_mut_ptr(),
            copy_len,
        );

        // Обновляем позицию (с выравниванием по 4 байта)
        ctrl.rx_pos = (ctrl.rx_pos + 4 + pkt_len + 3) & !3;
        if ctrl.rx_pos >= RX_BUFFER_SIZE {
            ctrl.rx_pos = 0;
        }

        // Уведомляем контроллер о новой позиции
        rtl_outw(ctrl.io_base + RTL_CAPR, (ctrl.rx_pos as u16).wrapping_sub(16));

        Some(copy_len)
    }
}
