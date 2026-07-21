use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};
use alloc::format;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let count = crate::drivers::pci::device_count();
    console.print_colored("PCI Devices: ", COLOR_CYAN);
    console.print_num(count);
    console.print("\n\n");

    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                unsafe {
                    let vendor = crate::drivers::pci::pci_read_u16_config(bus, dev, func, 0x00);
                    if vendor == 0xFFFF { if func == 0 { break; } continue; }
                    let device = crate::drivers::pci::pci_read_u16_config(bus, dev, func, 0x02);
                    let hdr = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0E);
                    let class = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0B);
                    let sub = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0A);

                    let name = match (class, sub) {
                        (0x01, 0x06) => "AHCI/SATA",
                        (0x01, 0x08) => "NVMe",
                        (0x01, _) => "Storage",
                        (0x02, 0x00) => "Ethernet",
                        (0x02, _) => "Network",
                        (0x03, 0x00) => "VGA",
                        (0x06, 0x00) => "Host Bridge",
                        (0x06, 0x01) => "ISA Bridge",
                        (0x06, 0x04) => "PCI Bridge",
                        (0x0C, 0x03) => "USB",
                        _ => "Other",
                    };

                    let line = format!("  {:02X}:{:02X}.{}  {:04X}:{:04X}  {}\n",
                        bus, dev, func, vendor, device, name);
                    console.print(&line);

                    if func == 0 && hdr & 0x80 == 0 { break; }
                }
            }
        }
    }
    CmdResult::Ok
}
