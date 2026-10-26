use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;
use crate::fs::File as _;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.len() < 2 {
        console.print_colored("Usage: cp <source> <destination>\n", COLOR_RED);
        return CmdResult::Ok;
    }

    let source = args[0];
    let dest = args[1];

    // Читаем исходный файл (VFS → fallback ramfs).
    match fs::read_file(source) {
        Ok(data) => {
            // Запись сначала через VFS: так копирование работает и на
            // смонтированных томах (FAT32/ext2), а не только в ramfs.
            let written = match fs::vfs_create(dest) {
                Ok(mut file) => file.write(&data).map(|_| ()),
                // Путь обслуживает реальная ФС — ошибку (read-only, нет места,
                // нет каталога) надо показать, иначе копия молча осядет в ramfs
                // и пользователь увидит «Copied», хотя на носителе ничего нет.
                Err(e) if fs::mount_point_of(dest).is_some() => Err(e),
                Err(_) => {
                    if fs::create_file(dest).is_err() {
                        // Файл уже существует — это нормально, перезапишем.
                    }
                    fs::write_file(dest, &data)
                }
            };

            match written {
                Ok(()) => {
                    console.print_colored("Copied: ", COLOR_GREEN);
                    console.print(source);
                    console.print_colored(" -> ", COLOR_YELLOW);
                    console.print(dest);
                    console.print(" (");
                    console.print_num(data.len());
                    console.print(" bytes)\n");
                }
                Err(e) => {
                    console.print_colored("Error writing: ", COLOR_RED);
                    console.print(&format!("{}", e));
                    console.put_char('\n');
                }
            }
        }
        Err(e) => {
            console.print_colored("Error reading: ", COLOR_RED);
            console.print(&format!("{}", e));
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}