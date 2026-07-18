use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN};
use crate::kernel;
use crate::version;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Vibra OS - System Information\n", COLOR_CYAN);
    console.print_colored("==============================\n\n", COLOR_CYAN);
    
    console.print_colored("OS Version:      ", COLOR_YELLOW);
    console.print(version::OS_VERSION);
    console.print(" \"");
    console.print(version::OS_CODENAME);
    console.print("\"\n");
    
    console.print_colored("Kernel:          ", COLOR_YELLOW);
    console.print("v");
    console.print(version::KERNEL_VERSION);
    console.print(" \"");
    console.print(version::KERNEL_CODENAME);
    console.print("\"\n");
    
    console.print_colored("Architecture:    ", COLOR_YELLOW);
    console.print(version::ARCHITECTURE);
    console.print("\n");
    
    console.print_colored("Author:          ", COLOR_YELLOW);
    console.print_colored(version::AUTHOR, COLOR_GREEN);
    console.print("\n");
    
    console.print_colored("License:         ", COLOR_YELLOW);
    console.print(version::LICENSE);
    console.print("\n");
    
    console.print_colored("Year:            ", COLOR_YELLOW);
    console.print(version::YEAR);
    console.print("\n");
    
    console.print("\n");
    console.print_colored("Kernel Subsystems:\n", COLOR_CYAN);
    console.print_colored("  Devices:  ", COLOR_YELLOW);
    console.print_num(kernel::registry::device_count());
    console.print("\n");
    
    console.print_colored("  Drivers:  ", COLOR_YELLOW);
    console.print_num(kernel::registry::driver_count());
    console.print("\n");
    
    console.print_colored("  Modules:  ", COLOR_YELLOW);
    console.print_num(kernel::registry::module_count());
    console.print(" (built-in)\n");
    
    console.print("\n");
    console.print_colored("Built-in Modules:\n", COLOR_CYAN);
    console.print_colored("  • vfs         ", COLOR_GREEN);
    console.print("- Virtual File System\n");
    console.print_colored("  • console     ", COLOR_GREEN);
    console.print("- Console Manager\n");
    console.print_colored("  • scheduler   ", COLOR_GREEN);
    console.print("- Task Scheduler (stub)\n");
    
    CmdResult::Ok
}