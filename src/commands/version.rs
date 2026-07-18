use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN};
use crate::version;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let show_kernel = args.first().map(|s| *s == "kernel").unwrap_or(false);
    
    if show_kernel {
        console.print_colored("Vibra Kernel v", COLOR_CYAN);
        console.print_colored(version::KERNEL_VERSION, COLOR_CYAN);
        console.print(" \"");
        console.print_colored(version::KERNEL_CODENAME, COLOR_CYAN);
        console.print("\"\n");
        console.print("  Architecture : ");
        console.print(version::ARCHITECTURE);
        console.print("\n");
        console.print("  Type         : Modular Monolithic\n");
        console.print("  Bootloader   : Limine (UEFI)\n");
    } else {
        console.print_colored("Vibra OS v", COLOR_CYAN);
        console.print_colored(version::OS_VERSION, COLOR_CYAN);
        console.print(" \"");
        console.print_colored(version::OS_CODENAME, COLOR_CYAN);
        console.print("\"\n");
        console.print("  Kernel: v");
        console.print(version::KERNEL_VERSION);
        console.print(" \"");
        console.print(version::KERNEL_CODENAME);
        console.print("\"\n");
        console.print("  Tip: try 'version kernel' for details\n");
    }
    CmdResult::Ok
}