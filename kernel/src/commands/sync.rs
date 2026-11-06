// sync — записать отложенные данные на носитель.
//
// FAT32 пишет инкрементально, но перед выключением или выдёргиванием
// носителя полезно явно попросить ФС сбросить метаданные. Заодно
// сохраняются boot- и shell-логи.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_YELLOW};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Syncing filesystems...\n", COLOR_YELLOW);

    let mounts = crate::fs::mount_entries();
    let mut errors = 0usize;

    for (path, _name, readonly) in mounts {
        if readonly {
            continue;
        }
        if let Err(error) = crate::fs::vfs_sync(&path) {
            console.print_colored("sync: ", crate::framebuffer::COLOR_RED);
            console.print(&path);
            console.print(": ");
            console.print(&alloc::format!("{}", error));
            console.put_char('\n');
            errors += 1;
        }
    }

    // Логи живут отдельно от VFS-таблицы: сбрасываем их всегда.
    crate::boot_log::flush_all_files();
    crate::boot_log::checkpoint_now();

    if errors == 0 {
        console.print_colored("sync: done\n", COLOR_GREEN);
    } else {
        console.print_colored("sync: completed with errors\n", crate::framebuffer::COLOR_RED);
    }

    CmdResult::Ok
}
