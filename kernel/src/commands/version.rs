use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN};
use crate::version;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Vibra Kernel v", COLOR_CYAN);
    console.print_colored(version::KERNEL_VERSION, COLOR_CYAN);
    console.print(" ");
    console.print_colored(version::KERNEL_CODENAME, COLOR_CYAN);
    console.print("\n");
    console.print("  Architecture : ");
    console.print(version::ARCHITECTURE);
    console.print("\n  Type         : Experimental modular monolith\n");
    console.print("  Bootloader   : Limine (UEFI, x86_64)\n");
    CmdResult::Ok
}
