use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_YELLOW, COLOR_CYAN};
use crate::fs;
use alloc::string::String;

use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    // Без аргумента — текущий каталог, а не всегда корень. Раньше `tree`
    // из /mnt показывал дерево всего диска, потому что путь жёстко был "/".
    let path = if args.is_empty() {
        fs::get_current_dir()
    } else {
        fs::resolve_path(&fs::get_current_dir(), args[0])
    };

    console.print_colored(&path, COLOR_CYAN);
    console.print("\n");

    let mut dirs = 0usize;
    let mut files = 0usize;

    let completed = print_tree(console, &path, "", &mut dirs, &mut files);

    console.print("\n");
    console.print_num(dirs);
    console.print(" directories, ");
    console.print_num(files);
    console.print(" files\n");
    if !completed {
        console.print_colored("[cancelled]\n", COLOR_YELLOW);
    }

    CmdResult::Ok
}

fn print_tree(
    console: &mut Console,
    path: &str,
    prefix: &str,
    dirs: &mut usize,
    files: &mut usize,
) -> bool {
    // Ctrl+C / Ctrl+Z должны прерывать обход: на большом разделе `tree`
    // иначе нельзя остановить.
    if crate::poll_cancel() {
        return false;
    }

    let entries = fs::list_dir(path);
    let count = entries.len();

    for (i, entry) in entries.iter().enumerate() {
        if crate::poll_cancel() {
            return false;
        }
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
                if !print_tree(
                    console,
                    &sub_path,
                    &format!("{}{}", prefix, new_prefix),
                    dirs,
                    files,
                ) {
                    return false;
                }
            }
            fs::FileType::File | fs::FileType::Symlink | fs::FileType::Device => {
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

    true
}
