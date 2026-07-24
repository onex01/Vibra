// ICMP — Internet Control Message Protocol (RFC 792)
// Echo Request (type 8) / Echo Reply (type 0) для ping.

use crate::println;
use crate::timer;

/// ICMP заголовок (8 байт минимум)
#[repr(C, packed)]
pub struct IcmpHeader {
    pub type_code: u8,   // Тип (старший байт)
    pub code: u8,        // Код (младший байт)
    pub checksum: u16,
    pub id: u16,
    pub sequence: u16,
}

pub const ICMP_ECHO_REQUEST: u8 = 8;
pub const ICMP_ECHO_REPLY: u8 = 0;

/// Вычислить ICMP чек сумму (RFC 1071)
fn icmp_checksum(data: &[u8]) -> u16 {
    super::ip::internet_checksum(data)
}

/// Отправить ICMP Echo Request
pub fn send_echo_request(dst_ip: [u8; 4], id: u16, sequence: u16, payload: &[u8]) {
    let mut icmp_data = alloc::vec![0u8; 8 + payload.len()];

    let hdr = unsafe { &mut *(icmp_data.as_mut_ptr() as *mut IcmpHeader) };
    hdr.type_code = ICMP_ECHO_REQUEST;
    hdr.code = 0;
    hdr.checksum = 0;
    hdr.id = id.to_be();
    hdr.sequence = sequence.to_be();

    icmp_data[8..].copy_from_slice(payload);

    // Вычисляем чек сумму
    let csum = icmp_checksum(&icmp_data);
    hdr.checksum = csum.to_be();

    super::ip::send_ip(dst_ip, super::ip::IP_PROTO_ICMP, &icmp_data);
}

/// Отправить ICMP Echo Reply
fn send_echo_reply(dst_ip: [u8; 4], id: u16, sequence: u16, payload: &[u8]) {
    let mut icmp_data = alloc::vec![0u8; 8 + payload.len()];

    let hdr = unsafe { &mut *(icmp_data.as_mut_ptr() as *mut IcmpHeader) };
    hdr.type_code = ICMP_ECHO_REPLY;
    hdr.code = 0;
    hdr.checksum = 0;
    hdr.id = id.to_be();
    hdr.sequence = sequence.to_be();

    icmp_data[8..].copy_from_slice(payload);

    let csum = icmp_checksum(&icmp_data);
    hdr.checksum = csum.to_be();

    super::ip::send_ip(dst_ip, super::ip::IP_PROTO_ICMP, &icmp_data);
}

/// Обработать входящий ICMP пакет
pub fn handle_icmp(src_ip: [u8; 4], _dst_ip: [u8; 4], data: &[u8]) {
    if data.len() < 8 {
        return;
    }

    let hdr = unsafe { &*(data.as_ptr() as *const IcmpHeader) };
    let msg_type = hdr.type_code;
    let id = u16::from_be(hdr.id);
    let sequence = u16::from_be(hdr.sequence);
    let payload = &data[8..];

    match msg_type {
        ICMP_ECHO_REQUEST => {
            println!("[ICMP] Echo Request от {} id={} seq={}",
                super::ip::ip_to_str(src_ip), id, sequence);
            // Автоматический ответ
            send_echo_reply(src_ip, id, sequence, payload);
        }
        ICMP_ECHO_REPLY => {
            println!("[ICMP] Echo Reply от {} id={} seq={}",
                super::ip::ip_to_str(src_ip), id, sequence);
            set_reply_received(id, sequence);
        }
        _ => {
            println!("[ICMP] Неизвестный тип: {}", msg_type);
        }
    }
}

/// Ping — отправить N Echo Request и подождать ответов
pub fn ping(target_ip: [u8; 4], count: u32) {
    println!("[PING] {} ({} пакетов):", super::ip::ip_to_str(target_ip), count);

    let id = 0xBEEF;
    let mut received = 0u32;
    let mut min_rtt: u64 = u64::MAX;
    let mut max_rtt: u64 = 0;
    let mut total_rtt: u64 = 0;

    for seq in 0..count {
        let t_start = timer::current_ticks();
        let tps = timer::ticks_per_second();

        // Формируем payload с временем отправки
        let mut payload = [0u8; 64];
        payload[0] = 0xDE;
        payload[1] = 0xAD;
        payload[2] = 0xBE;
        payload[3] = 0xEF;
        let time_ms = if tps > 0 { t_start * 1000 / tps } else { t_start };
        payload[4] = (time_ms & 0xFF) as u8;
        payload[5] = ((time_ms >> 8) & 0xFF) as u8;
        payload[6] = ((time_ms >> 16) & 0xFF) as u8;
        payload[7] = ((time_ms >> 24) & 0xFF) as u8;

        send_echo_request(target_ip, id, seq as u16, &payload);

        // Ожидаем ответ с таймаутом 2000 мс
        let timeout_ms = 2000u64;
        let mut got_reply = false;
        let wait_start = timer::current_ticks();

        loop {
            // Поллим e1000 для получения пакетов
            super::poll_recv_once();

            let now = timer::current_ticks();
            let elapsed_ms = if tps > 0 { (now - wait_start) * 1000 / tps } else { now - wait_start };
            if elapsed_ms >= timeout_ms {
                break;
            }

            // Проверяем, есть ли ответ с нужным id/seq
            if icmp_pending_reply(id, seq as u16) {
                got_reply = true;
                break;
            }

            core::hint::spin_loop();
        }

        let t_end = timer::current_ticks();
        let rtt_ms = if tps > 0 { (t_end - t_start) * 1000 / tps } else { t_end - t_start };

        if got_reply {
            println!("  Ответ от {}: байт=64 время={}мс seq={}",
                super::ip::ip_to_str(target_ip), rtt_ms, seq);
            received += 1;
            if rtt_ms < min_rtt { min_rtt = rtt_ms; }
            if rtt_ms > max_rtt { max_rtt = rtt_ms; }
            total_rtt += rtt_ms;
        } else {
            println!("  Превышен тайм-аут для {}: seq={}",
                super::ip::ip_to_str(target_ip), seq);
        }

        // Небольшая пауза между пингами
        timer::sleep_ms(100);
    }

    println!("\n--- Статистика {} ---", super::ip::ip_to_str(target_ip));
    println!("Пакетов: отправлено={}, получено={}, потери={}",
        count, received, count - received);

    if received > 0 {
        let avg_rtt = total_rtt / (received as u64);
        println!("RTT: мин={}мс, макс={}мс, среднее={}мс",
            min_rtt, max_rtt, avg_rtt);
    }
}

/// Флаг: получен ли ICMP Reply с заданным id/sequence
static ICMP_REPLY_ID: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);
static ICMP_REPLY_SEQ: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);
static ICMP_REPLY_AVAIL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

/// Проверить, получен ли ответ
pub fn icmp_pending_reply(id: u16, seq: u16) -> bool {
    if ICMP_REPLY_AVAIL.load(Ordering::Relaxed) {
        let rid = ICMP_REPLY_ID.load(Ordering::Relaxed);
        let rseq = ICMP_REPLY_SEQ.load(Ordering::Relaxed);
        if rid == id && rseq == seq {
            ICMP_REPLY_AVAIL.store(false, Ordering::Relaxed);
            return true;
        }
    }
    false
}

/// Установить флаг получения ответа (вызывается из handle_icmp)
fn set_reply_received(id: u16, seq: u16) {
    ICMP_REPLY_ID.store(id, Ordering::Relaxed);
    ICMP_REPLY_SEQ.store(seq, Ordering::Relaxed);
    ICMP_REPLY_AVAIL.store(true, Ordering::Relaxed);
}
