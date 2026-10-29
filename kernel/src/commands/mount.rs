use super::CmdResult;
use alloc::format;
use alloc::boxed::Box;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN, COLOR_RED};
use crate::fs;
use crate::fs::FileSystem;

/// Разобрать аргумент-идентификатор диска: `sda1` или `2`.
fn parse_disk_id(arg: &str) -> Option<usize> {
    if let Ok(id) = arg.parse::<usize>() { return Some(id); }
    let mgr = fs::get_disk_manager().lock();
    mgr.find_by_name(arg)
}

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
        {
            let mgr = fs::get_disk_manager().lock();
            if mgr.count() == 0 {
                console.print_colored("  No block devices found\n", COLOR_RED);
            } else {
                console.print("  ID  NAME   TYPE       SIZE       FS\n");
                for (i, e) in mgr.entries().iter().enumerate() {
                    console.print(&format!(
                        "  {:<3} {:<6} {:<10} {:>6} MB  {}\n",
                        i, e.name, e.kind, e.size_mb(), e.fs_type
                    ));
                }
            }
        }
        
        console.print("\n");
        console.print_colored("Usage:\n", COLOR_CYAN);
        console.print("  mount                                - show mounted filesystems\n");
        console.print("  mount <path> auto <disk_id|name>     - auto-detect (read-only)\n");
        console.print("  mount <path> fat32 <disk_id|name>    - mount FAT32 (read-only)\n");
        console.print("  mount <path> fat32 <disk_id|name> rw - mount FAT32 read/write\n");
        console.print("  mount <path> ext <disk_id|name>      - mount EXT2/3/4 (read-only)\n");
        console.print("  mount <path> ntfs <disk_id|name>     - mount NTFS (read-only)\n");
        console.print("  mount <path> ramfs                   - mount RAMFS\n");
        console.print("  mount /dev/sdX1 /mnt [rw]            - Linux-style alias\n");
        console.print("\n");
        console.print_colored("Write access to physical media is opt-in: pass 'rw' explicitly.\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    // Принимаем и Linux-подобную форму: `mount /dev/sdb1 /mnt [rw]` и
    // короткую `mount /mnt sdb1`. Раньше первый аргумент всегда считался
    // путём, поэтому `/dev/sdb1 /mnt` давал «unknown filesystem /mnt».
    let mut normalized: alloc::vec::Vec<&str> = args.to_vec();
    let first_is_dev = args[0].starts_with("/dev/") || parse_disk_id(args[0]).is_some();
    if first_is_dev && args.len() >= 2 && args[1].starts_with('/') {
        let dev = args[0].trim_start_matches("/dev/");
        let path = args[1];
        let fs_type = args.get(2).copied().unwrap_or("auto");
        normalized.clear();
        normalized.push(path);
        normalized.push(fs_type);
        normalized.push(dev);
        if args.iter().any(|a| *a == "rw") {
            normalized.push("rw");
        }
    } else if args.len() == 2
        && args[0].starts_with('/')
        && !args[0].starts_with("/dev/")
        && parse_disk_id(args[1]).is_some()
    {
        normalized.clear();
        normalized.push(args[0]);
        normalized.push("auto");
        normalized.push(args[1]);
    }
    let args: &[&str] = &normalized;

    if args.len() >= 2 {
        let path = args[0];
        let fs_type = args[1];

        // ramfs не требует диска.
            if fs_type == "ramfs" {
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
                return CmdResult::Ok;
            }

            // Остальные типы требуют блочное устройство.
            let Some(dev) = args.get(2).copied() else {
                console.print_colored(
                    &format!("Usage: mount {} {} <disk_id|name>\n", path, fs_type),
                    COLOR_RED,
                );
                return CmdResult::Ok;
            };

            let Some(disk_id) = parse_disk_id(dev) else {
                console.print_colored(&format!("Unknown disk: {}\n", dev), COLOR_RED);
                return CmdResult::Ok;
            };

            console.print(&format!("  Mounting {} at {} from disk {}... ", fs_type, path, dev));

            // exFAT пока не поддерживается: честно сообщаем, а не отдаём
            // неинформативное "Operation not supported".
            let detected = {
                let mgr = fs::get_disk_manager().lock();
                mgr.info(disk_id).map(|(_, _, _, t)| t)
            };
            if detected == Some("exfat") {
                console.print_colored("SKIPPED (exFAT is not supported)\n", COLOR_RED);
                console.print_colored("  Use a FAT32/NTFS volume or convert the disk.\n", COLOR_YELLOW);
                return CmdResult::Ok;
            }

            // Запись на физический носитель — только по явному `rw`.
            let write = args.iter().skip(3).any(|a| *a == "rw");

            let result = match fs_type {
                "auto" => {
                    if write {
                        fs::mount_auto_rw(path, disk_id).map(|s| format!("OK ({}, rw)", s))
                    } else {
                        fs::mount_auto(path, disk_id).map(|s| format!("OK ({}, ro)", s))
                    }
                }
                "fat32" | "vfat" => match fs::get_disk_manager().lock().open(disk_id) {
                    Some(disk) if write => {
                        fs::mount_fat32_rw(path, disk).map(|_| format!("OK (fat32, rw)"))
                    }
                    Some(disk) => fs::mount_fat32(path, disk).map(|_| format!("OK (fat32, ro)")),
                    None => Err(fs::FsError::NotFound),
                },
                "ext" | "ext2" | "ext3" | "ext4" => {
                    match fs::get_disk_manager().lock().open(disk_id) {
                        Some(disk) => fs::mount_ext(path, disk, true).map(|_| format!("OK (ext, ro)")),
                        None => Err(fs::FsError::NotFound),
                    }
                }
                "ntfs" => match fs::get_disk_manager().lock().open(disk_id) {
                    Some(disk) => fs::mount_ntfs(path, disk).map(|_| format!("OK (ntfs, ro)")),
                    None => Err(fs::FsError::NotFound),
                },
                "exfat" => {
                    console.print_colored("not supported: exFAT (use FAT32/NTFS)\n", COLOR_RED);
                    return CmdResult::Ok;
                }
                _ => {
                    console.print_colored(&format!("unknown type '{}'\n", fs_type), COLOR_RED);
                    return CmdResult::Ok;
                }
            };

            match result {
                Ok(msg) => console.print_colored(&format!("{}\n", msg), COLOR_GREEN),
                Err(e) => console.print_colored(&format!("FAILED: {}\n", e), COLOR_RED),
            }
    }

    CmdResult::Ok
}

