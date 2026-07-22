#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use vibra_kernel as kernel;

mod gui;
mod commands;

/// Команды vibra ОС (GUI + расширенные)
fn register_os_commands() {
    kernel::commands::register_command(kernel::commands::Command {
        name: "cpuid",
        help: "show CPU information",
        func: commands::cpuid::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "memmap",
        help: "show memory map",
        func: commands::memmap::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "diskinfo",
        help: "show AHCI disk info",
        func: commands::diskinfo::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "lsusb",
        help: "list USB controllers",
        func: commands::lsusb::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "desktop",
        help: "launch graphical desktop",
        func: commands::desktop::run,
    });
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Инициализация ядра (hardware, memory, drivers, scheduler)
    let bc = kernel::init();

    // Регистрируем OS-команды (GUI, desktop, extended)
    register_os_commands();

    // Запускаем shell с базовыми + OS командами
    kernel::shell_loop(bc);
}
