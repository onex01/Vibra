use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_CYAN, COLOR_WHITE};
use crate::fs;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let show_all = args.iter().any(|a| *a == "-a" || *a == "-la" || *a == "-al");
    let long_format = args.iter().any(|a| *a == "-l" || *a == "-la" || *a == "-al");

    // `ls [path...]`: все не-опции — пути. По умолчанию — текущий каталог.
    let mut paths: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|a| String::from(*a))
        .collect();
    if paths.is_empty() {
        paths.push(fs::get_current_dir());
    }

    let multiple = paths.len() > 1;
    for path in &paths {
        if multiple {
            console.print_colored(&format!("\n{}:\n", path), COLOR_CYAN);
        }
        list_one(console, path, long_format, show_all);
    }

    CmdResult::Ok
}

fn list_one(console: &mut Console, path: &str, long_format: bool, show_all: bool) {
    let mut entries = fs::list_dir(path);

    if show_all {
        // Linux-подобные `.` и `..` для навигации/проверки.
        let dot = fs::DirEntry {
            name: String::from("."),
            file_type: fs::FileType::Directory,
            size: 0,
            permissions: fs::Permissions::new(0o755),
            uid: 0,
            gid: 0,
        };
        let dotdot = fs::DirEntry {
            name: String::from(".."),
            file_type: fs::FileType::Directory,
            size: 0,
            permissions: fs::Permissions::new(0o755),
            uid: 0,
            gid: 0,
        };
        entries.insert(0, dot);
        entries.insert(1, dotdot);
    }

    if long_format {
        console.print_colored("total ", COLOR_YELLOW);
        console.print_num(entries.len());
        console.print("\n");

        for entry in &entries {
            match entry.file_type {
                fs::FileType::Directory => console.print_colored("d", COLOR_GREEN),
                fs::FileType::File => console.print_colored("-", COLOR_WHITE),
                fs::FileType::Symlink => console.print_colored("l", COLOR_CYAN),
                fs::FileType::Device => console.print_colored("c", COLOR_YELLOW),
            }

            console.print(&entry.permissions.to_string());
            console.print(" 1 root root ");

            if entry.size < 10 {
                console.print("    ");
            } else if entry.size < 100 {
                console.print("   ");
            } else if entry.size < 1000 {
                console.print("  ");
            } else {
                console.print(" ");
            }
            console.print_num(entry.size);

            console.print(" Jan  1 00:00 ");

            match entry.file_type {
                fs::FileType::Directory => console.print_colored(&entry.name, COLOR_GREEN),
                _ => console.print(&entry.name),
            }
            console.put_char('\n');
        }
    } else {
        let mut col = 0;
        for entry in &entries {
            match entry.file_type {
                fs::FileType::Directory => {
                    console.print_colored(&entry.name, COLOR_GREEN);
                    console.print("/  ");
                }
                fs::FileType::File | fs::FileType::Symlink | fs::FileType::Device => {
                    console.print(&entry.name);
                    console.print("  ");
                }
            }
            col += 1;
            if col >= 4 {
                console.put_char('\n');
                col = 0;
            }
        }
        if col > 0 {
            console.put_char('\n');
        }
    }
}