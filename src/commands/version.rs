use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let show_kernel = args.first().map(|s| *s == "kernel").unwrap_or(false);
    
    if show_kernel {
        console.print_colored("Vibra Kernel v", COLOR_CYAN);
        console.print(env!("CARGO_PKG_VERSION"));
        console.print("\n");
        console.print("  Architecture : x86_64\n");
        console.print("  Type         : Modular Monolithic\n");
        console.print("  Bootloader   : Limine (UEFI)\n");
    } else {
        console.print_colored("Vibra OS v0.4 \"Photon\"\n", COLOR_CYAN);
        console.print("  Kernel: ");
        console.print(env!("CARGO_PKG_VERSION"));
        console.print("\n");
        console.print("  Tip: try 'version kernel' for details\n");
    }
    CmdResult::Ok
}