// Паспорт машины и USB-инвентарь для `diag hw` / `diag usb`.

use crate::framebuffer::{Console, COLOR_YELLOW};
use alloc::format;
use alloc::string::String;

/// Паспорт машины: SMBIOS, список ACPI-таблиц, флаги FADT.
///
/// Первый запрос при отладке на незнакомом железе: видно, есть ли EC (ECDT),
/// есть ли 8042, какие таблицы прошивка вообще отдала.
pub fn diag_hw(console: &mut Console) {
    console.print_colored("[SMBIOS]\n", COLOR_YELLOW);
    match crate::acpi::smbios::read() {
        Some(info) => {
            console.print(&format!(
                "  system:  {} {} ({})\n",
                empty(&info.system_manufacturer),
                empty(&info.system_product),
                empty(&info.system_version)
            ));
            console.print(&format!(
                "  board:   {} {}\n",
                empty(&info.board_manufacturer),
                empty(&info.board_product)
            ));
            console.print(&format!(
                "  bios:    {} {} ({})\n",
                empty(&info.bios_vendor),
                empty(&info.bios_version),
                empty(&info.bios_date)
            ));
            console.print(&format!(
                "  cpu:     {} {} ({} cores / {} threads)\n",
                empty(&info.cpu_vendor),
                empty(&info.cpu_version),
                info.cpu_cores,
                info.cpu_threads
            ));
            if let Some(serial) = non_empty(&info.system_serial) {
                console.print(&format!("  serial:  {}\n", serial));
            }

            let total: u64 = info.memory.iter().map(|module| module.size_mb).sum();
            console.print(&format!(
                "  memory:  {} module(s), {} MiB total\n",
                info.memory.len(),
                total
            ));
            for module in &info.memory {
                console.print(&format!(
                    "    {:<12} {:>6} MiB  {:<6} {:>5} MT/s  {} {}\n",
                    empty(&module.locator),
                    module.size_mb,
                    module.memory_type,
                    module.speed_mts,
                    module.manufacturer,
                    module.part_number
                ));
            }
        }
        None => console.print("  entry point not found (SMBIOS unavailable)\n"),
    }

    console.print_colored("[ACPI]\n", COLOR_YELLOW);
    let tables = crate::acpi::table_list();
    if tables.is_empty() {
        console.print("  no tables\n");
    } else {
        console.print(&format!("  {} table(s):\n", tables.len()));
        for (signature, phys, length) in &tables {
            let name = core::str::from_utf8(signature).unwrap_or("????");
            console.print(&format!(
                "    {} @ {:#010x} ({} bytes)\n",
                name, phys, length
            ));
        }
    }

    let acpi = crate::acpi::get();
    match acpi.as_ref() {
        Some(info) => {
            console.print(&format!(
                "  LAPIC:   {:#x}\n  IOAPIC:  {:#x} (id {})\n",
                info.lapic_addr.unwrap_or(0),
                info.io_apic_addr.unwrap_or(0),
                info.io_apic_id.unwrap_or(0)
            ));
            console.print(&format!(
                "  MCFG:    {}\n  HPET:    {}\n",
                info.mcfg_base
                    .map(|base| format!("{:#x}", base))
                    .unwrap_or_else(|| String::from("absent")),
                info.hpet_base
                    .map(|base| format!("{:#x}", base))
                    .unwrap_or_else(|| String::from("absent"))
            ));
            console.print(&format!(
                "  FADT boot flags: {:#06x}, 8042: {}\n",
                info.boot_arch_flags.unwrap_or(0),
                if crate::acpi::has_8042() {
                    "present"
                } else {
                    "absent"
                }
            ));
        }
        None => console.print("  ACPI not initialized\n"),
    }

    console.print_colored("[BUS]\n", COLOR_YELLOW);
    let usb_controllers = crate::drivers::pci::find_all_devices_by_class(0x0C, 0x03).len();
    console.print(&format!(
        "  USB controllers: {}\n  SATA active ports: {}\n",
        usb_controllers,
        crate::drivers::ahci::active_ports().len()
    ));
}

fn empty(value: &str) -> &str {
    if value.is_empty() {
        "-"
    } else {
        value
    }
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
