// IP — Internet Protocol (RFC 791)
// Базовый IP-слой: заголовок 20 байт, контрольная сумма.
// Поддерживает только ICMP (protocol 1) для ping.

use crate::println;

/// Заголовок IP пакета (20 байт)
#[repr(C, packed)]
pub struct IpHeader {
    pub ver_ihl: u8,        // Версия (4) + IHL (5) = 0x45
    pub dscp_ecn: u8,       // DSCP + ECN
    pub total_len: u16,     // Длина всего пакета
    pub identification: u16,
    pub flags_fragment: u16,
    pub ttl: u8,            // Time To Live
    pub protocol: u8,       // 1 = ICMP, 6 = TCP, 17 = UDP
    pub checksum: u16,
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
}

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

/// Вычислить интернет-чек сумму (RFC 1071)
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    // Суммируем 16-битные слова
    while i + 1 < data.len() {
        let word = ((data[i] as u32) << 8) | (data[i + 1] as u32);
        sum += word;
        i += 2;
    }

    // Если нечётное количество байт, дополняем нулём
    if i < data.len() {
        let word = (data[i] as u32) << 8;
        sum += word;
    }

    // Сворачиваем carry
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    // Инвертируем
    (!sum as u16)
}

/// Разобрать IP адрес из строки "10.0.2.2"
pub fn parse_ip(s: &str) -> [u8; 4] {
    let mut ip = [0u8; 4];
    for (i, p) in s.split('.').enumerate() {
        if i >= 4 { break; }
        ip[i] = p.parse().unwrap_or(0);
    }
    ip
}

/// IP адрес в строку
pub fn ip_to_str(ip: [u8; 4]) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// Отправить IP пакет
pub fn send_ip(dst_ip: [u8; 4], protocol: u8, payload: &[u8]) {
    let src_ip = super::get_local_ip();
    let total_len = 20 + payload.len();

    let mut ip_header = [0u8; 20];
    let hdr = unsafe { &mut *(ip_header.as_mut_ptr() as *mut IpHeader) };
    hdr.ver_ihl = 0x45;
    hdr.dscp_ecn = 0;
    hdr.total_len = (total_len as u16).to_be();
    hdr.identification = 0x1234;
    hdr.flags_fragment = 0x4000; // Don't Fragment
    hdr.ttl = 64;
    hdr.protocol = protocol;
    hdr.checksum = 0;
    hdr.src_ip = src_ip;
    hdr.dst_ip = dst_ip;

    // Вычисляем чек сумму
    let csum = internet_checksum(&ip_header);
    hdr.checksum = csum.to_be();

    // Формируем полный пакет
    let mut packet = alloc::vec![0u8; total_len];
    packet[..20].copy_from_slice(&ip_header);
    packet[20..].copy_from_slice(payload);

    // Определяем MAC назначения через ARP
    let gateway = super::get_gateway();
    let target_ip = if is_same_subnet(dst_ip, src_ip, super::get_subnet_mask()) {
        dst_ip
    } else {
        gateway
    };

    let mac = match super::arp::resolve(target_ip, super::get_local_mac(), src_ip) {
        Some(m) => m,
        None => {
            println!("[IP] Не удалось разрешить MAC для {}", ip_to_str(target_ip));
            return;
        }
    };

    super::send_ethernet(mac, 0x0800, &packet);
}

/// Проверить, находятся ли два IP в одной подсети
fn is_same_subnet(ip1: [u8; 4], ip2: [u8; 4], mask: [u8; 4]) -> bool {
    for i in 0..4 {
        if (ip1[i] & mask[i]) != (ip2[i] & mask[i]) {
            return false;
        }
    }
    true
}

/// Обработать входящий IP пакет
pub fn handle_ip(eth_payload: &[u8]) {
    if eth_payload.len() < 20 {
        return;
    }

    let hdr = unsafe { &*(eth_payload.as_ptr() as *const IpHeader) };
    let ver_ihl = hdr.ver_ihl;
    let version = (ver_ihl >> 4) & 0x0F;
    let ihl = (ver_ihl & 0x0F) as usize * 4;
    let total_len = u16::from_be(hdr.total_len) as usize;
    let protocol = hdr.protocol;
    let src_ip = hdr.src_ip;
    let dst_ip = hdr.dst_ip;

    if version != 4 {
        println!("[IP] Неподдерживаемая версия: {}", version);
        return;
    }

    if eth_payload.len() < total_len {
        return;
    }

    // Проверяем, адресован ли нам
    let local_ip = super::get_local_ip();
    if dst_ip != local_ip && dst_ip != [255, 255, 255, 255] {
        return; // Не наш пакет
    }

    // Проверяем чек сумму
    let mut hdr_copy = [0u8; 20];
    hdr_copy.copy_from_slice(&eth_payload[..20]);
    let saved_csum = u16::from_be(hdr.checksum);
    hdr_copy[10] = 0;
    hdr_copy[11] = 0;
    let calc_csum = internet_checksum(&hdr_copy);
    if calc_csum != saved_csum {
        println!("[IP] Ошибка контрольной суммы");
        return;
    }

    let data = &eth_payload[ihl..total_len];

    match protocol {
        IP_PROTO_ICMP => {
            super::icmp::handle_icmp(src_ip, dst_ip, data);
        }
        _ => {
            println!("[IP] Протокол не поддерживается: {}", protocol);
        }
    }
}
