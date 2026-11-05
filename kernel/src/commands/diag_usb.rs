// USB-инвентарь для `diag usb`.

use crate::framebuffer::{Console, COLOR_YELLOW};
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

    console.print_colored("[root ports]\n", COLOR_YELLOW);
    let ports = crate::drivers::usb::xhci::ports();
    if ports.is_empty() {
        console.print("  none\n");
    }
    for port in &ports {
        // Компактно: только подключённые порты и всё, что уже перечислено.
        if !port.connected && port.device.is_none() {
            continue;
        }
        console.print(&format!(
            "  ctrl{} port{:<2} {} {} {} speed {} ({})",
            port.controller,
            port.port,
            if port.connected { "connected" } else { "empty" },
            if port.enabled { "enabled" } else { "disabled" },
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
