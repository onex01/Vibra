use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN};
use crate::version;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Vibra OS - About\n", COLOR_CYAN);
    console.print_colored("================\n\n", COLOR_CYAN);
    
    console.print_colored("Project: ", COLOR_YELLOW);
    console.print("Vibra OS\n");
    
    console.print_colored("Version: ", COLOR_YELLOW);
    console.print(version::OS_VERSION);
    console.print(" \"");
    console.print(version::OS_CODENAME);
    console.print("\"\n");
    
    console.print_colored("Kernel:  ", COLOR_YELLOW);
    console.print("v");
    console.print(version::KERNEL_VERSION);
    console.print(" \"");
    console.print(version::KERNEL_CODENAME);
    console.print("\"\n");
    
    console.print_colored("Created: ", COLOR_YELLOW);
    console.print(version::YEAR);
    console.print("\n");
    
    console.print_colored("Author:  ", COLOR_YELLOW);
    console.print_colored(version::AUTHOR, COLOR_GREEN);
    console.print("\n");
    
    console.print_colored("License: ", COLOR_YELLOW);
    console.print(version::LICENSE);
    console.print("\n\n");
    
    console.print_colored("Description:\n", COLOR_YELLOW);
    console.print("  ");
    console.print(version::DESCRIPTION);
    console.print("\n");
    console.print("  Features: modular kernel, graphical console,\n");
    console.print("  RamFS, shell with tab-completion and history.\n\n");
    
    console.print_colored("Built with:\n", COLOR_YELLOW);
    console.print("  - Rust (nightly)\n");
    console.print("  - Limine bootloader\n");
    console.print("  - QEMU emulator\n");
    
    CmdResult::Ok
}