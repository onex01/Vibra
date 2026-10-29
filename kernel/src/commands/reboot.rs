// reboot — перезагрузка.
//
// Раньше команда сразу писала в 0x92 и 0x64/0xFE. На реальном железе этого
// недостаточно: часть чипсетов требует FADT RESET_REG или 0xCF9, а i8042
// может быть занят. Теперь используется общий ACPI-reset с несколькими
// fallback-путями, а последним средством остаётся triple fault.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Rebooting...\n", COLOR_YELLOW);

    // Сохраняем лог в единственный файл и на VIBRALOG (если подключён).
    crate::boot_log::flush_to_file("/var/log/boot.log");
    crate::boot_log::checkpoint();

    if let Err(error) = crate::acpi::power::reset() {
        console.print_colored("ACPI reset failed: ", COLOR_RED);
        console.print(error);
        console.print_colored("; falling back to triple fault\n", COLOR_RED);
    }

    // Последний шанс перезагрузить машину: загружаем пустой IDT и вызываем
    // #GP → double fault → triple fault. Это лучше вечного hlt.
    unsafe {
        let idt = [0u8; 10];
        core::arch::asm!("lidt [{}]", in(reg) &idt, options(readonly, nostack));
        core::arch::asm!("int3", options(nostack));
    }

    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}