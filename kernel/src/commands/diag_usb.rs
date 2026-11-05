// USB-инвентарь для `diag usb`.

use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};
use alloc::format;

/// Контроллеры, устройства, накопители и состояние приёмника boot-лога.
pub fn diag_usb(console: &mut Console) {
    console.print_colored("[UHCI]\n", COLOR_YELLOW);
    console.print(&format!(
        "  controllers: {}\n  keyboards:   {}\n",
        crate::drivers::usb::uhci::controller_count(),
        crate::drivers::usb::uhci::keyboard_count()
    ));
    for device in crate::drivers::usb::uhci::devices() {
        console.print(&format!(
            "  ctrl{} port{} {:04x}:{:04x} addr {} {} (max packet {})\n",
            device.controller,
            device.port,
            device.vendor,
            device.product,
            device.address,
            if device.low_speed { "low-speed" } else { "full-speed" },
            device.max_packet
        ));
    }

    console.print_colored("[xHCI]\n", COLOR_YELLOW);
    console.print(&format!(
        "  controllers: {}\n  keyboard:    {}\n",
        crate::drivers::usb::xhci::controller_count(),
        crate::drivers::usb::xhci::keyboard_status()
    ));

    // Контроллеры USB, которых у нас нет. Ищем по class 0x0C / subclass 0x03
    // (USB) и различаем по prog_if: 0x00 UHCI, 0x10 OHCI, 0x20 EHCI,
    // 0x30 xHCI. Раньше здесь стоял subclass 0x20, поэтому EHCI на ноутбуке
    // не находился и печаталось «none», хотя в логе он есть (8086:9CA6).
    console.print_colored("[controllers without driver]\n", COLOR_YELLOW);
    let mut missing = 0usize;
    for device in crate::drivers::pci::find_all_devices_by_class(0x0C, 0x03) {
        let prog_if = device.prog_if;
        let name = match prog_if {
            0x00 => "UHCI (USB 1.1)",
            0x10 => "OHCI (USB 1.1)",
            0x20 => "EHCI (USB 2.0)",
            0x30 => "xHCI (USB 3.x)",
            _ => "USB (unknown prog_if)",
        };
        // UHCI и xHCI у нас есть; остальные — нет.
        if prog_if == 0x00 || prog_if == 0x30 {
            continue;
        }
        console.print(&format!(
            "  [{:02x}:{:02x}.{}] {:04x}:{:04x} prog_if {:#04x} -> {}\n",
            device.bus, device.dev, device.func,
            device.vendor_id, device.device_id, prog_if, name
        ));
        missing += 1;
    }
    if missing == 0 {
        console.print("  none\n");
    } else {
        console.print_colored(
            "  -> a drive in one of these ports cannot appear: no driver\n",
            COLOR_RED,
        );
    }

    // Расширенные возможности: какие протоколы есть и что с legacy handoff.
    // Без этого «0 connected» невозможно истолковать: непонятно, какие порты
    // вообще USB3, а какие USB2, и не держит ли контроллер прошивка.
    if let Some(caps) = crate::drivers::usb::xhci::extended_caps(0) {
        console.print_colored("[xECP]\n", COLOR_YELLOW);
        if caps.xecp_offset == 0 {
            console.print("  none\n");
        } else {
            console.print(&format!(
                "  offset {:#x}, legacy support {}, bios_owned {}, os_owned {}\n",
                caps.xecp_offset,
                caps.legacy_support,
                caps.bios_owned,
                caps.os_owned
            ));
            for (major, minor, from, count, name) in &caps.protocols {
                console.print(&format!(
                    "  {}.{} {} ports {}-{}\n",
                    major,
                    minor,
                    name,
                    from,
                    from.saturating_add(count.saturating_sub(1))
                ));
            }
            if caps.bios_owned {
                console.print_colored(
                    "  -> firmware still owns the controller: ports may read as empty\n",
                    COLOR_RED,
                );
            }
        }
    }

    // Какой порт к какому протоколу относится: по xECP видно, где USB3,
    // а где USB2. Без этого при 15 портах непонятно, куда втыкать флешку.
    let protocols = crate::drivers::usb::xhci::extended_caps(0)
        .map(|c| c.protocols)
        .unwrap_or_default();
    let protocol_of = |port: u8| -> &'static str {
        for (major, _minor, from, count, _name) in &protocols {
            let to = from.saturating_add(count.saturating_sub(1));
            if port >= *from && port <= to {
                return if *major >= 3 { "usb3" } else { "usb2" };
            }
        }
        "?"
    };

    console.print_colored("[root ports]\n", COLOR_YELLOW);
    let ports = crate::drivers::usb::xhci::ports();
    if ports.is_empty() {
        console.print("  none\n");
    }
    // Печатаем ВСЕ порты, включая пустые: иначе «флешка не видна» невозможно
    // отличить от «порт вообще не видит подключения», а это разные причины
    // (второе — другой контроллер, например EHCI, драйвера которого нет).
    if !ports.is_empty() {
        let connected = ports.iter().filter(|p| p.connected).count();
        console.print(&format!(
            "  {} port(s), {} connected\n",
            ports.len(),
            connected
        ));
    }
    for port in &ports {
        console.print(&format!(
            "  ctrl{} port{:<2} {:<4} {:#010x} {} {} {} speed {} ({})",
            port.controller,
            port.port,
            protocol_of(port.port),
            port.portsc,
            if port.connected { "conn" } else { "empty" },
            if port.enabled { "en" } else { "dis" },
            if port.powered { "powered" } else { "unpowered" },
            port.speed,
            crate::drivers::usb::xhci::speed_name(port.speed)
        ));
        match port.device {
            Some((vendor, product, kind)) => console.print(&format!(
                " -> {:04x}:{:04x} {}\n",
                vendor, product, kind
            )),
            None => console.print(" -> not enumerated\n"),
        }
    }

    console.print_colored("[devices]\n", COLOR_YELLOW);
    let devices = crate::drivers::usb::xhci::devices();
    if devices.is_empty() {
        console.print("  none\n");
    }
    for device in &devices {
        console.print(&format!(
            "  ctrl{} slot{:<2} port{:<2} {:04x}:{:04x} class {:02x}/{:02x}/{:02x} speed {} ({}) -> {}\n",
            device.controller,
            device.slot,
            device.port,
            device.vendor,
            device.product,
            device.class,
            device.subclass,
            device.protocol,
            device.speed,
            crate::drivers::usb::xhci::speed_name(device.speed),
            device.kind
        ));
    }

    console.print_colored("[mass storage]\n", COLOR_YELLOW);
    let disks = crate::drivers::usb::storage::disks();
    if disks.is_empty() {
        console.print("  none\n");
    }
    for disk in &disks {
        console.print(&format!(
            "  ctrl{} slot{}: {} {} ({} MiB, {} sectors, {} sec/cmd, {} speed, burst {})\n",
            disk.controller,
            disk.slot,
            disk.model(),
            disk.revision,
            disk.size_mb(),
            disk.sectors,
            disk.sectors_per_cmd,
            crate::drivers::usb::xhci::speed_name(
                crate::drivers::usb::xhci::storage_devices()
                    .iter()
                    .find(|d| d.slot == disk.slot)
                    .map(|d| d.speed)
                    .unwrap_or(0)
            ),
            crate::drivers::usb::xhci::storage_devices()
                .iter()
                .find(|d| d.slot == disk.slot)
                .map(|d| d.max_burst)
                .unwrap_or(0)
        ));
    }

    console.print_colored("[boot log sink]\n", COLOR_YELLOW);
    if crate::boot_log::usb::is_attached() {
        console.print(&format!(
            "  VIBRALOG: attached, {} bytes written, ring {} bytes total\n",
            crate::boot_log::usb::written(),
            crate::boot_log::total_len()
        ));
    } else {
        console.print(&format!(
            "  VIBRALOG: not attached (feature {}), ring {} bytes total\n",
            if cfg!(feature = "boot-log-to-usb") {
                "on"
            } else {
                "off"
            },
            crate::boot_log::total_len()
        ));
    }
}
