use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Memory Map:\n", crate::framebuffer::COLOR_CYAN);
    console.print("  Type                    Base              Size\n");

    if let Some(mm) = crate::MEMORY_MAP_REQUEST.response() {
        for entry in mm.entries() {
            let type_str = match entry.type_ {
                limine::memmap::MEMMAP_USABLE => "Usable",
                limine::memmap::MEMMAP_RESERVED => "Reserved",
                limine::memmap::MEMMAP_ACPI_RECLAIMABLE => "ACPI Reclaim",
                limine::memmap::MEMMAP_ACPI_NVS => "ACPI NVS",
                limine::memmap::MEMMAP_BAD_MEMORY => "Bad Memory",
                limine::memmap::MEMMAP_BOOTLOADER_RECLAIMABLE => "Boot Reclaim",
                limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES => "Kernel/Mod",
                limine::memmap::MEMMAP_FRAMEBUFFER => "Framebuffer",
                _ => "Unknown",
            };
            console.print("  ");
            let pad = 23 - type_str.len().min(23);
            console.print(type_str);
            for _ in 0..pad { console.print(" "); }

            let base = alloc::format!("{:#018x}", entry.base);
            console.print(&base);
            console.print("  ");

            let size_kb = entry.length / 1024;
            let size_str = if size_kb >= 1024 * 1024 {
                alloc::format!("{} GB", size_kb / (1024 * 1024))
            } else if size_kb >= 1024 {
                alloc::format!("{} MB", size_kb / 1024)
            } else {
                alloc::format!("{} KB", size_kb)
            };
            console.print(&size_str);
            console.print("\n");
        }
    } else {
        console.print_colored("  Memory map not available\n", crate::framebuffer::COLOR_RED);
    }

    CmdResult::Ok
}
