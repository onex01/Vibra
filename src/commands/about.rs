use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW, COLOR_GREEN};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Vibra OS - About\n", COLOR_CYAN);
    console.print_colored("================\n\n", COLOR_CYAN);
    
    console.print_colored("Project: ", COLOR_YELLOW);
    console.print("Vibra OS\n");
    
    console.print_colored("Version: ", COLOR_YELLOW);
    console.print("0.4 \"Photon\"\n");
    
    console.print_colored("Kernel:  ", COLOR_YELLOW);
    console.print("v0.4.0\n");
    
    console.print_colored("Created: ", COLOR_YELLOW);
    console.print("2026-07-18\n");
    
    console.print_colored("Author:  ", COLOR_YELLOW);
    console.print_colored("OneX01\n", COLOR_GREEN);
    
    console.print_colored("License: ", COLOR_YELLOW);
    console.print("MIT\n\n");
    
    console.print_colored("Description:\n", COLOR_YELLOW);
    console.print("  Vibra is a hobby operating system written in Rust.\n");
    console.print("  Features: modular kernel, graphical console,\n");
    console.print("  RamFS, shell with tab-completion and history.\n\n");
    
    console.print_colored("Built with:\n", COLOR_YELLOW);
    console.print("  - Rust (nightly)\n");
    console.print("  - Limine bootloader\n");
    console.print("  - QEMU emulator\n");
    
    CmdResult::Ok
}