// USB-инвентарь для `diag usb`.

use crate::framebuffer::{Console, COLOR_YELLOW};
use alloc::format;

/// Контроллеры, устройства, накопители и состояние приёмника boot-лога.
pub fn diag_usb(console: &mut Console) {
    console.print_colored("[xHCI]\n", COLOR_YELLOW);
    console.print(&format!(
        "  controllers: {}\n  keyboard:    {}\n",
        crate::drivers::usb::xhci::controller_count(),
        crate::drivers::usb::xhci::keyboard_status()
    ));

    console.print_colored("[devices]\n", COLOR_YELLOW);
    let devices = crate::drivers::usb::xhci::devices();
    if devices.is_empty() {
        console.print("  none\n");
    }
    for device in &devices {
        console.print(&format!(
            "  ctrl{} slot{:<2} port{:<2} {:04x}:{:04x} class {:02x}/{:02x}/{:02x} speed {} -> {}\n",
            device.controller,
            device.slot,
            device.port,
            device.vendor,
            device.product,
            device.class,
            device.subclass,
            device.protocol,
            device.speed,
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
            "  ctrl{} slot{}: {} {} ({} MiB, {} sectors, {} sec/cmd)\n",
            disk.controller,
            disk.slot,
            disk.model(),
            disk.revision,
            disk.size_mb(),
            disk.sectors,
            disk.sectors_per_cmd
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
