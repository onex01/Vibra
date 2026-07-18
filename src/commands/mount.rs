use super::CmdResult;
use alloc::format;
use alloc::boxed::Box;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN, COLOR_RED};
use crate::fs;
use crate::fs::FileSystem;

/// Команда mount - список или монтирование файловых систем
pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        // Показать список смонтированных ФС
        console.print_colored("Mounted filesystems:\n", COLOR_CYAN);
        console.print_colored("====================\n", COLOR_CYAN);
        
        let mounts = fs::list_mounts();
        if mounts.is_empty() {
            console.print_colored("  No filesystems mounted\n", COLOR_RED);
        } else {
            for mount in mounts {
                console.print(&format!("  {}\n", mount));
            }
        }
        
        console.print("\n");
        console.print_colored("Available disk devices:\n", COLOR_YELLOW);
        let count = fs::get_disk_manager().lock().count();
        console.print(&format!("  Total disks: {}\n", count));
        
        console.print("\n");
        console.print_colored("Usage:\n", COLOR_CYAN);
        console.print("  mount                          - show mounted filesystems\n");
        console.print("  mount <path> fat32 <disk_id>   - mount FAT32 disk\n");
        console.print("  mount <path> ext [ro|rw]       - mount EXT2/3/4 disk\n");
        console.print("  mount <path> ramfs             - mount RAMFS\n");
    } else {
        console.print_colored("Mounting filesystems...\n", COLOR_YELLOW);
        
        // Простая реализация для теста
        let path = args.first().copied().unwrap_or("/");
        
        if args.len() >= 2 {
            let fs_type = args[1];
            match fs_type {
                "ramfs" => {
                    console.print(&format!("  Mounting RAMFS at {}... ", path));
                    let mut ramfs = fs::RamFs::new();
                    if ramfs.mount().is_ok() {
                        if fs::mount_fs(path, Box::new(ramfs), false).is_ok() {
                            console.print_colored("OK\n", COLOR_GREEN);
                        } else {
                            console.print_colored("FAILED (already mounted)\n", COLOR_RED);
                        }
                    } else {
                        console.print_colored("FAILED\n", COLOR_RED);
                    }
                }
                "fat32" => {
                    console.print(&format!("  Mounting FAT32 at {}... ", path));
                    console.print_colored("NOT YET IMPLEMENTED\n", COLOR_YELLOW);
                    console.print("  (Need real disk driver for FAT32)\n");
                }
                "ext" => {
                    console.print(&format!("  Mounting EXT at {}... ", path));
                    console.print_colored("NOT YET IMPLEMENTED\n", COLOR_YELLOW);
                    console.print("  (Need real disk driver for EXT)\n");
                }
                _ => {
                    console.print_colored("Unknown filesystem type: ", COLOR_RED);
                    console.print(fs_type);
                    console.print("\n");
                }
            }
        } else {
            console.print_colored("Usage: mount <path> <filesystem_type>\n", COLOR_RED);
        }
    }
    
    CmdResult::Ok
}
