// diskinfo — показать информацию о найденных дисках AHCI.

use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW};
use alloc::format;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let disks = vibra_kernel::drivers::ahci::get_disk_info();

    if disks.is_empty() {
        console.print_colored("Диски не найдены (AHCI)\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    console.print_colored("Найдено дисков: ", COLOR_CYAN);
    console.print_num(disks.len());
    console.print("\n\n");

    for (i, disk) in disks.iter().enumerate() {
        // Размер в MiB
        let size_sectors = disk.total_sectors;
        let size_bytes = size_sectors * disk.sector_size as u64;
        let size_mib = size_bytes / (1024 * 1024);
        let size_gib = size_bytes / (1024 * 1024 * 1024);

        console.print_colored(&format!("  Диск{}: {} ", i, disk.model), COLOR_YELLOW);
        console.print_colored(&format!("({} MiB, {} GiB)\n", size_mib, size_gib), COLOR_CYAN);

        console.print_colored(&format!("    Серийный: {}\n", disk.serial), COLOR_CYAN);
        console.print_colored(&format!("    Секторов: {}, размер сектора: {}\n",
            disk.total_sectors, disk.sector_size), COLOR_CYAN);
    }

    CmdResult::Ok
}
