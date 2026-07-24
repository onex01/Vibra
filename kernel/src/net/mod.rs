// Network — базовый TCP/IP стек Vibra OS.
// Ethernet + ARP + IP + ICMP.
// Использует e1000 драйвер для передачи кадров.

pub mod arp;
pub mod ip;
pub mod icmp;

use crate::println;
use spin::Mutex;
use core::sync::atomic::{AtomicBool, Ordering};

/// Сетевой интерфейс
pub struct NetInterface {
    mac: [u8; 6],
    ip: [u8; 4],
    gateway: [u8; 4],
    subnet_mask: [u8; 4],
}

static NET_INTERFACE: Mutex<Option<NetInterface>> = Mutex::new(None);
static NET_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Инициализация сети
pub fn init(mac: [u8; 6]) {
    // IP адрес QEMU user-mode networking: 10.0.2.15
    // Шлюз (DHCP server): 10.0.2.2
    let ip = [10, 0, 2, 15];
    let gateway = [10, 0, 2, 2];
    let subnet_mask = [255, 255, 255, 0];

    println!("[NET] Инициализация сетевого интерфейса...");
    println!("[NET] MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
    println!("[NET] IP: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    println!("[NET] Шлюз: {}.{}.{}.{}", gateway[0], gateway[1], gateway[2], gateway[3]);
    println!("[NET] Маска: {}.{}.{}.{}", subnet_mask[0], subnet_mask[1], subnet_mask[2], subnet_mask[3]);

    *NET_INTERFACE.lock() = Some(NetInterface {
        mac,
        ip,
        gateway,
        subnet_mask,
    });
    NET_INITIALIZED.store(true, Ordering::SeqCst);
    println!("[NET] Сетевой интерфейс инициализирован");
}

/// Получить MAC адрес локального интерфейса
pub fn get_local_mac() -> [u8; 6] {
    match NET_INTERFACE.lock().as_ref() {
        Some(iface) => iface.mac,
        None => [0; 6],
    }
}

/// Получить IP адрес локального интерфейса
pub fn get_local_ip() -> [u8; 4] {
    match NET_INTERFACE.lock().as_ref() {
        Some(iface) => iface.ip,
        None => [0; 4],
    }
}

/// Получить адрес шлюза
pub fn get_gateway() -> [u8; 4] {
    match NET_INTERFACE.lock().as_ref() {
        Some(iface) => iface.gateway,
        None => [0; 4],
    }
}

/// Получить маску подсети
pub fn get_subnet_mask() -> [u8; 4] {
    match NET_INTERFACE.lock().as_ref() {
        Some(iface) => iface.subnet_mask,
        None => [0; 4],
    }
}

/// Проверить, инициализирована ли сеть
pub fn is_initialized() -> bool {
    NET_INITIALIZED.load(Ordering::SeqCst)
}

/// Отправить Ethernet кадр
pub fn send_ethernet(dst: [u8; 6], ether_type: u16, payload: &[u8]) {
    let src = get_local_mac();

    // Ethernet заголовок: 14 байт
    let mut frame = alloc::vec![0u8; 14 + payload.len()];
    frame[0..6].copy_from_slice(&dst);
    frame[6..12].copy_from_slice(&src);
    frame[12..14].copy_from_slice(&ether_type.to_be_bytes());
    frame[14..].copy_from_slice(payload);

    crate::drivers::e1000::send_packet(&frame);
}

/// Получить данные из ARP таблицы (для ifconfig)
pub fn get_arp_table() -> alloc::vec::Vec<arp::ArpEntry> {
    let table = arp::ARP_TABLE.lock();
    table.clone()
}

/// Обработать входящий Ethernet кадр
pub fn handle_ethernet(frame: &[u8]) {
    if frame.len() < 14 {
        return;
    }

    let dst_mac = [frame[0], frame[1], frame[2], frame[3], frame[4], frame[5]];
    let src_mac = [frame[6], frame[7], frame[8], frame[9], frame[10], frame[11]];
    let ether_type = u16::from_be_bytes([frame[12], frame[13]]);

    // Проверяем, адресован ли нам (unicast или broadcast)
    let local_mac = get_local_mac();
    let is_broadcast = dst_mac == [0xFF; 6];
    let is_ours = dst_mac == local_mac;

    if !is_broadcast && !is_ours {
        return;
    }

    let payload = &frame[14..];

    match ether_type {
        0x0806 => { // ARP
            arp::handle_arp(payload);
        }
        0x0800 => { // IPv4
            ip::handle_ip(payload);
        }
        _ => {
            // Игнорируем другие протоколы
        }
    }
}

/// Один цикл опроса e1000 на наличие входящих пакетов
pub fn poll_recv_once() {
    if !NET_INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    let mut buf = [0u8; 2048];
    while let Some(len) = crate::drivers::e1000::recv_packet(&mut buf) {
        handle_ethernet(&buf[..len]);
    }
}

/// Полный цикл опроса (вызывается из shell loop)
pub fn poll_recv() {
    if !NET_INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    let mut buf = [0u8; 2048];
    if let Some(len) = crate::drivers::e1000::recv_packet(&mut buf) {
        handle_ethernet(&buf[..len]);
    }
}

// ===== Статический буфер для приёма =====
static mut NET_BUF: [u8; 2048] = [0; 2048];

/// Опрос с использованием статического буфера
pub fn poll_recv_static() {
    if !NET_INITIALIZED.load(Ordering::Relaxed) {
        return;
    }

    unsafe {
        if let Some(len) = crate::drivers::e1000::recv_packet(&mut NET_BUF) {
            handle_ethernet(&NET_BUF[..len]);
        }
    }
}
