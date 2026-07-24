// ARP — Address Resolution Protocol (RFC 826)
// Разрешение IP → MAC адрес.
// Таблица ARP хранит известные пары (IP, MAC).
// Запросы отправляются на broadcast FF:FF:FF:FF:FF:FF.

use alloc::vec::Vec;
use spin::Mutex;
use crate::println;

/// Формат ARP пакета (Ethernet/IPv4)
#[repr(C, packed)]
pub struct ArpPacket {
    pub hw_type: u16,      // Тип канала: 1 = Ethernet
    pub proto_type: u16,   // Тип протокола: 0x0800 = IPv4
    pub hw_len: u8,        // Длина MAC: 6
    pub proto_len: u8,     // Длина IP: 4
    pub opcode: u16,       // Операция: 1 = запрос, 2 = ответ
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_mac: [u8; 6],
    pub target_ip: [u8; 4],
}

pub const ARP_OP_REQUEST: u16 = 1;
pub const ARP_OP_REPLY: u16 = 2;
pub const ARP_HTYPE_ETHERNET: u16 = 1;
pub const ARP_PTYPE_IPV4: u16 = 0x0800;
pub const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

/// Запись в таблице ARP
#[derive(Clone, Debug)]
pub struct ArpEntry {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
}

/// Таблица ARP
pub static ARP_TABLE: Mutex<Vec<ArpEntry>> = Mutex::new(Vec::new());

/// Ожидание ответа ARP (busy-wait с таймаутом)
static ARP_PENDING_IP: Mutex<Option<[u8; 4]>> = Mutex::new(None);
static ARP_RESOLVED_MAC: Mutex<Option<[u8; 6]>> = Mutex::new(None);

/// Добавить запись в ARP таблицу
pub fn add_entry(ip: [u8; 4], mac: [u8; 6]) {
    let mut table = ARP_TABLE.lock();
    // Проверяем, нет ли уже такой записи
    for entry in table.iter_mut() {
        if entry.ip == ip {
            entry.mac = mac;
            return;
        }
    }
    table.push(ArpEntry { ip, mac });
    println!("[ARP] Новая запись: {}.{}.{}.{} -> {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        ip[0], ip[1], ip[2], ip[3],
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

/// Разрешить IP → MAC (с таймаутом)
pub fn resolve(target_ip: [u8; 4], src_mac: [u8; 6], src_ip: [u8; 4]) -> Option<[u8; 6]> {
    // Сначала проверяем таблицу
    {
        let table = ARP_TABLE.lock();
        for entry in table.iter() {
            if entry.ip == target_ip {
                return Some(entry.mac);
            }
        }
    }

    // Отправляем ARP запрос
    println!("[ARP] Запрос для {}.{}.{}.{}", target_ip[0], target_ip[1], target_ip[2], target_ip[3]);
    send_arp_request(target_ip, src_mac, src_ip);

    // Ожидаем ответ (busy-wait)
    *ARP_PENDING_IP.lock() = Some(target_ip);
    *ARP_RESOLVED_MAC.lock() = None;

    let timeout_ms = 3000u64;
    let start = crate::timer::current_ticks();
    let tps = crate::timer::ticks_per_second();

    loop {
        // Проверяем, получен ли ответ
        if let Some(mac) = *ARP_RESOLVED_MAC.lock() {
            *ARP_PENDING_IP.lock() = None;
            return Some(mac);
        }

        // Таймаут
        let elapsed = crate::timer::current_ticks();
        let elapsed_ms = if tps > 0 { (elapsed - start) * 1000 / tps } else { elapsed - start };
        if elapsed_ms >= timeout_ms {
            break;
        }

        core::hint::spin_loop();
    }

    *ARP_PENDING_IP.lock() = None;
    None
}

/// Сформировать и отправить ARP запрос
pub fn send_arp_request(target_ip: [u8; 4], src_mac: [u8; 6], src_ip: [u8; 4]) {
    let mut packet = [0u8; 28]; // ARP payload

    // Заполняем ARP заголовок
    let arp = unsafe { &mut *(packet.as_mut_ptr() as *mut ArpPacket) };
    arp.hw_type = ARP_HTYPE_ETHERNET.to_be();
    arp.proto_type = ARP_PTYPE_IPV4.to_be();
    arp.hw_len = 6;
    arp.proto_len = 4;
    arp.opcode = ARP_OP_REQUEST.to_be();
    arp.sender_mac = src_mac;
    arp.sender_ip = src_ip;
    arp.target_mac = [0x00; 6]; // неизвестен
    arp.target_ip = target_ip;

    // Отправляем через Ethernet (ether_type 0x0806)
    super::send_ethernet(BROADCAST_MAC, 0x0806, &packet);
}

/// Сформировать и отправить ARP ответ
pub fn send_arp_reply(target_mac: [u8; 6], target_ip: [u8; 4],
                      src_mac: [u8; 6], src_ip: [u8; 4]) {
    let mut packet = [0u8; 28];

    let arp = unsafe { &mut *(packet.as_mut_ptr() as *mut ArpPacket) };
    arp.hw_type = ARP_HTYPE_ETHERNET.to_be();
    arp.proto_type = ARP_PTYPE_IPV4.to_be();
    arp.hw_len = 6;
    arp.proto_len = 4;
    arp.opcode = ARP_OP_REPLY.to_be();
    arp.sender_mac = src_mac;
    arp.sender_ip = src_ip;
    arp.target_mac = target_mac;
    arp.target_ip = target_ip;

    super::send_ethernet(target_mac, 0x0806, &packet);
}

/// Обработать входящий ARP пакет
pub fn handle_arp(payload: &[u8]) {
    if payload.len() < 28 {
        return;
    }

    let arp = unsafe { &*(payload.as_ptr() as *const ArpPacket) };
    let opcode = u16::from_be(arp.opcode);
    let sender_ip = arp.sender_ip;
    let sender_mac = arp.sender_mac;
    let target_ip = arp.target_ip;
    let target_mac = arp.target_mac;

    let local_ip = super::get_local_ip();

    match opcode {
        ARP_OP_REQUEST => {
            // Проверяем, адресован ли нам
            if target_ip == local_ip {
                println!("[ARP] Запрос от {}.{}.{}.{}", sender_ip[0], sender_ip[1], sender_ip[2], sender_ip[3]);
                // Добавляем отправителя в таблицу
                add_entry(sender_ip, sender_mac);
                // Отправляем ответ
                let mac = super::get_local_mac();
                send_arp_reply(sender_mac, sender_ip, mac, local_ip);
            }
        }
        ARP_OP_REPLY => {
            println!("[ARP] Ответ от {}.{}.{}.{}", sender_ip[0], sender_ip[1], sender_ip[2], sender_ip[3]);
            add_entry(sender_ip, sender_mac);

            // Проверяем, ждём ли мы этот ответ
            let pending = *ARP_PENDING_IP.lock();
            if pending == Some(sender_ip) {
                *ARP_RESOLVED_MAC.lock() = Some(sender_mac);
            }
        }
        _ => {
            println!("[ARP] Неизвестная операция: {}", opcode);
        }
    }
}
