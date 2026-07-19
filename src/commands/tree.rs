use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_YELLOW, COLOR_CYAN};
use crate::fs;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let path = if args.is_empty() {
        String::from("/")
    } else {
        let mut s = String::new();
        s.push_str(args[0]);
        s
    };

    console.print_colored(&path, COLOR_CYAN);
    console.print("\n");

    let mut dirs = 0usize;
    let mut files = 0usize;

    print_tree(console, &path, "", &mut dirs, &mut files);

    console.print("\n");
    console.print_num(dirs);
    console.print(" directories, ");
    console.print_num(files);
    console.print(" files\n");

    CmdResult::Ok
}

fn print_tree(console: &mut Console, path: &str, prefix: &str, dirs: &mut usize, files: &mut usize) {
    let entries = fs::list_dir(path);
    let count = entries.len();

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let new_prefix = if is_last { "    " } else { "│   " };

        match entry.file_type {
            fs::FileType::Directory => {
                *dirs += 1;
                console.print(prefix);
                console.print_colored(connector, COLOR_YELLOW);
                console.print_colored(&entry.name, COLOR_GREEN);
                console.print("/\n");

                // Рекурсивно показываем содержимое
                let mut sub_path = String::new();
                sub_path.push_str(path);
                if !path.ends_with('/') {
                    sub_path.push('/');
                }
                sub_path.push_str(&entry.name);

                print_tree(console, &sub_path, &format!("{}{}", prefix, new_prefix), dirs, files);
            }
            fs::FileType::File => {
                *files += 1;
                console.print(prefix);
                console.print(connector);
                console.print(&entry.name);
                if entry.size > 0 {
                    console.print(" (");
                    console.print_num(entry.size);
                    console.print(" B)");
                }
                console.print("\n");
            }
        }
    }
}
