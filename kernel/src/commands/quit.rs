// quit — выключение питания.
//
// Раньше команда писала `0x2000` в порт `0x604` (это PM1a_CNT **только у
// QEMU q35**) и `0x03` в `0x92` (это ACPI **reset**, а не poweroff). На
// реальном железе она поэтому либо ничего не делала, либо перезагружала
// машину. Теперь выключение идёт через ACPI S5 (`acpi::power::power_off`),
// а порты эмуляторов остались только как резервный путь.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};
use crate::println;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Shutting down...\n", COLOR_YELLOW);

    // Сохраняем лог до выключения питания: после S5 прочитать его будет
    // нечем, а последние строки — самые важные для диагностики.
    crate::boot_log::flush_to_file("/var/log/boot.log");
    crate::boot_log::checkpoint();

    match crate::acpi::power::power_off() {
        Err(reason) => {
            console.print_colored(
                &alloc::format!("Poweroff failed: {}\n", reason),
                COLOR_RED,
            );
            console.print_colored(
                "ACPI S5, q35/i440fx/VirtualBox ports and ACPI reset all failed.\n",
                COLOR_RED,
            );
            console.print_colored("System halted. Hold the power button to turn off.\n", COLOR_YELLOW);
            println!("[QUIT] poweroff failed: {}", reason);
        }
        // power_off() при успехе не возвращается — сюда попасть нельзя.
        Ok(()) => {}
    }

    loop {
        crate::interrupts::halt();
    }
}
