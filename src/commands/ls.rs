use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_CYAN, COLOR_WHITE};
use crate::fs;
use alloc::string::String;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let show_all = args.iter().any(|a| *a == "-a" || *a == "-la" || *a == "-al");
    let long_format = args.iter().any(|a| *a == "-l" || *a == "-la" || *a == "-al");

    let path = if args.is_empty() {
        fs::get_current_dir()
    } else {
        let mut s = String::new();
        for a in args {
            if !a.starts_with('-') {
                s.push_str(a);
                break;
            }
        }
        if s.is_empty() { fs::get_current_dir() } else { s }
    };

    let entries = fs::list_dir(&path);

    if long_format {
        console.print_colored("total ", COLOR_YELLOW);
        console.print_num(entries.len());
        console.print("\n");

        for entry in &entries {
            match entry.file_type {
                fs::FileType::Directory => {
                    console.print_colored("drwxr-xr-x", COLOR_GREEN);
                }
                fs::FileType::File => {
                    console.print_colored("-rw-r--r--", COLOR_WHITE);
                }
            }
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
                fs::FileType::Directory => {
                    console.print_colored(&entry.name, COLOR_GREEN);
                }
                fs::FileType::File => {
                    console.print(&entry.name);
                }
            }
            console.put_char('\n');
        }
    } else {
        // Short format: файлы с размером, директории без
        let mut col = 0;
        for entry in &entries {
            match entry.file_type {
                fs::FileType::Directory => {
                    console.print_colored(&entry.name, COLOR_GREEN);
                    console.print("/  ");
                }
                fs::FileType::File => {
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

    CmdResult::Ok
}
