use super::CmdResult;
use alloc::format;
use alloc::boxed::Box;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN, COLOR_RED};
use crate::fs;

/// Команда test-disk - создание и тест диска для FAT32/EXT
pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let cmd = args.first().copied().unwrap_or("help");
    
    match cmd {
        "help" => {
            console.print_colored("Disk testing commands:\n", COLOR_CYAN);
            console.print("  test-disk ram <size_mb>    - create RAM disk (for FAT32/EXT testing)\n");
            console.print("  test-disk list             - list available disks\n");
            console.print("  test-disk info <id>        - show disk info\n");
            console.print("  test-disk read <id> <sec>  - read sector from disk\n");
        }
        "ram" => {
            if args.len() < 2 {
                console.print_colored("Usage: test-disk ram <size_mb>\n", COLOR_RED);
                return CmdResult::Ok;
            }
            let size: usize = match args[1].parse() {
                Ok(s) => s,
                Err(_) => {
                    console.print_colored("Invalid size\n", COLOR_RED);
                    return CmdResult::Ok;
                }
            };
            
            console.print(&format!("Creating RAM disk {} MB... ", size));
            let disk = fs::RamDisk::new(size);
            let mut disk_mgr = fs::get_disk_manager().lock();
            disk_mgr.add_disk(Box::new(disk));
            console.print_colored("OK\n", COLOR_GREEN);
            console.print(&format!("  Disk ID: {}\n", disk_mgr.count() - 1));
        }
        "list" => {
            let count = fs::get_disk_manager().lock().count();
            console.print(&format!("Total disks: {}\n", count));
            if count > 0 {
                console.print("  IDs: ");
                for i in 0..count {
                    console.print(&format!("{} ", i));
                }
                console.print("\n");
            }
        }
        "info" => {
            if args.len() < 2 {
                console.print_colored("Usage: test-disk info <disk_id>\n", COLOR_RED);
                return CmdResult::Ok;
            }
            let id: usize = match args[1].parse() {
                Ok(i) => i,
                Err(_) => {
                    console.print_colored("Invalid disk ID\n", COLOR_RED);
                    return CmdResult::Ok;
                }
            };
            
            let mut disk_mgr = fs::get_disk_manager().lock();

            if let Some(_disk) = disk_mgr.get_disk(id) {
                console.print_colored(&format!("Disk {} info:\n", id), COLOR_YELLOW);
                console.print("  Type: RamDisk (in-memory)\n");
                console.print("  Sector size: 512 bytes\n");
                // For now we don't know the full size without downcasting
                console.print_colored("  Status: Ready\n", COLOR_GREEN);
            } else {
                console.print_colored(&format!("Disk {} not found\n", id), COLOR_RED);
            }
        }
        _ => {
            console.print_colored(&format!("Unknown command: {}\n", cmd), COLOR_RED);
        }
    }
    
    CmdResult::Ok
}
