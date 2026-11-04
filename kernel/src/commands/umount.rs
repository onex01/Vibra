// umount — размонтировать файловую систему.
//
// Без этой команды неудачное или ненужное монтирование занимало путь
// навсегда: повторный `mount` в ту же точку отвечал «already exists»,
// а освободить точку было нечем.
//
// `umount <path>`   — размонтировать точку
// `umount -a`       — размонтировать всё, кроме корня
// `umount <path> -f` — то же, но ошибки игнорируются
//
// Корень `/` не размонтируется: на нём живёт ramfs с /dev, /proc и /cfg.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::fs;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() || args.iter().any(|a| *a == "-h" || *a == "--help") {
        console.print_colored("Usage: umount <path> | umount -a [-f]\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    let force = args.iter().any(|a| *a == "-f" || *a == "--force");

    let targets: Vec<String> = if args.iter().any(|a| *a == "-a") {
        fs::mount_entries()
            .into_iter()
            .map(|(path, _, _)| path)
            .filter(|path| path != "/")
            .collect()
    } else {
        args.iter()
            .filter(|a| !a.starts_with('-'))
            .map(|a| String::from(*a))
            .collect()
    };

    if targets.is_empty() {
        console.print_colored("umount: nothing to unmount\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    for target in targets {
        if target == "/" {
            console.print_colored("umount: cannot unmount /\n", COLOR_RED);
            continue;
        }
        match fs::vfs_unmount(&target) {
            Ok(()) => {
                console.print_colored("Unmounted ", COLOR_GREEN);
                console.print(&target);
                console.put_char('\n');
            }
            Err(error) => {
                console.print_colored("umount: ", COLOR_RED);
                console.print(&target);
                console.print(": ");
                console.print(&format!("{}", error));
                console.put_char('\n');
                if !force {
                    continue;
                }
            }
        }
    }

    CmdResult::Ok
}
