#![no_std]
#![no_main]

extern crate alloc;

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
        name: "gfx",
        help: "graphical demo (ESC/Ctrl+Z to exit)",
        func: commands::gfx_demo::run,
    });
    kernel::commands::register_command(kernel::commands::Command {
        name: "desktop",
        help: "launch graphical desktop",
        func: commands::desktop::run,
    });
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let bc = kernel::init();
    register_os_commands();
    // Boot into shell — GUI доступна через команду gfx/desktop
    kernel::shell_loop(bc);
}
