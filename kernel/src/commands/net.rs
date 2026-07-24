// Команды сети: ping, ifconfig

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW};

/// ping <ip> — ICMP ping
pub fn ping(args: &[&str], console: &mut Console) -> CmdResult {
    if !crate::net::is_initialized() {
        console.print_colored("[NET] Сеть не инициализирована\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    if args.is_empty() {
        console.print_colored("Использование: ping <ip-адрес>\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    let target_ip = crate::net::ip::parse_ip(args[0]);
    let count = if args.len() > 1 {
        args[1].parse().unwrap_or(4)
    } else {
        4
    };

    // Выполняем ping через println! для вывода
    crate::net::icmp::ping(target_ip, count);
    CmdResult::Ok
}

/// ifconfig — показать информацию о сети
pub fn ifconfig(_args: &[&str], console: &mut Console) -> CmdResult {
    if !crate::net::is_initialized() {
        console.print_colored("[NET] Сеть не инициализирована\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    let mac = crate::net::get_local_mac();
    let ip = crate::net::get_local_ip();
    let gw = crate::net::get_gateway();
    let mask = crate::net::get_subnet_mask();

    console.print_colored("eth0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>\n", COLOR_GREEN);
    console.print_colored("  MAC: ", COLOR_CYAN);
    console.print(&alloc::format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}\n",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]));
    console.print_colored("  IP: ", COLOR_CYAN);
    console.print(&alloc::format!("{}.{}.{}.{}\n", ip[0], ip[1], ip[2], ip[3]));
    console.print_colored("  Шлюз: ", COLOR_CYAN);
    console.print(&alloc::format!("{}.{}.{}.{}\n", gw[0], gw[1], gw[2], gw[3]));
    console.print_colored("  Маска: ", COLOR_CYAN);
    console.print(&alloc::format!("{}.{}.{}.{}\n", mask[0], mask[1], mask[2], mask[3]));
    console.print_colored("  Статус: ", COLOR_CYAN);
    console.print_colored("активен\n", COLOR_GREEN);

    // Показываем ARP таблицу
    let arp_table = crate::net::get_arp_table();
    if !arp_table.is_empty() {
        console.print_colored("\nARP таблица:\n", COLOR_CYAN);
        for entry in arp_table.iter() {
            console.print(&alloc::format!("  {}.{}.{}.{} -> {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}\n",
                entry.ip[0], entry.ip[1], entry.ip[2], entry.ip[3],
                entry.mac[0], entry.mac[1], entry.mac[2],
                entry.mac[3], entry.mac[4], entry.mac[5]));
        }
    }

    CmdResult::Ok
}
