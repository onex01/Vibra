use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN, COLOR_CYAN};
use crate::fs;
use alloc::string::String;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let path = if args.is_empty() {
        fs::get_current_dir()
    } else {
        let mut s = String::new();
        s.push_str(args[0]);
        s
    };

    console.print_colored("Directory: ", COLOR_YELLOW);
    console.print(&path);
    console.print("\n\n");

    // Показываем корень с подробной информацией
    if path == "/" {
        console.print_colored("Mode    Type  Size    Name\n", COLOR_CYAN);
        console.print_colored("------  ----  ------  ----\n", COLOR_CYAN);

        // Стандартные каталоги
        let dirs: [(&str, &str); 11] = [
            ("/bin",     "drwxr-xr-x  0  bin     "),
            ("/boot",    "drwxr-xr-x  0  boot    "),
            ("/dev",     "drwxr-xr-x  0  dev     "),
            ("/etc",     "drwxr-xr-x  0  etc     "),
            ("/home",    "drwxr-xr-x  0  home    "),
            ("/mnt",     "drwxr-xr-x  0  mnt     "),
            ("/proc",    "drwxr-xr-x  0  proc    "),
            ("/root",    "drwx------  0  root    "),
            ("/sys",     "drwxr-xr-x  0  sys     "),
            ("/tmp",     "drwxrwxrwx  0  tmp     "),
            ("/var",     "drwxr-xr-x  0  var     "),
        ];

        for (mode, name) in &dirs {
            console.print_colored(mode, COLOR_GREEN);
            console.print("  ");
            console.print(name);
            console.print("\n");
        }

        // Файлы в корне
        let files = fs::list_dir("/");
        for entry in &files {
            match entry.file_type {
                fs::FileType::Directory => {
                    // Уже показаны выше
                }
                fs::FileType::File => {
                    console.print("-rw-r--r--  ");
                    console.print_num(entry.size);
                    console.print("  ");
                    console.print(&entry.name);
                    console.print("\n");
                }
            }
        }
    } else {
        // Простой вывод для не-корневых каталогов
        let count = fs::fs_count();
        console.print_colored("Total: ", COLOR_YELLOW);
        console.print_num(count);
        console.print(" entries\n\n");

        for entry in fs::list_dir(&path) {
            match entry.file_type {
                fs::FileType::Directory => {
                    console.print_colored("[DIR] ", COLOR_GREEN);
                    console.print(&entry.name);
                }
                fs::FileType::File => {
                    console.print("      ");
                    console.print(&entry.name);
                    console.print(" (");
                    console.print_num(entry.size);
                    console.print(" bytes)");
                }
            }
            console.put_char('\n');
        }
    }
    CmdResult::Ok
}