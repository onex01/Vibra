use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_VIBRA_PROMPT};
use crate::version;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("\n");
    console.print_colored("     __     ___ _           \n", COLOR_CYAN);
    console.print_colored("     \\ \\   / (_) |__  _ __ __ _ \n", COLOR_CYAN);
    console.print_colored("      \\ \\ / /| | '_ \\| '__/ _` |\n", COLOR_CYAN);
    console.print_colored("       \\ V / | | |_) | | | (_| |\n", COLOR_CYAN);
    console.print_colored("        \\_/  |_|_.__/|_|  \\__,_|\n", COLOR_CYAN);
    console.print("\n");
    console.print_colored("    Vibra OS v", COLOR_VIBRA_PROMPT);
    console.print_colored(version::OS_VERSION, COLOR_VIBRA_PROMPT);
    console.print(" \"");
    console.print_colored(version::OS_CODENAME, COLOR_VIBRA_PROMPT);
    console.print("\" | Kernel v");
    console.print_colored(version::KERNEL_VERSION, COLOR_VIBRA_PROMPT);
    console.print(" \"");
    console.print_colored(version::KERNEL_CODENAME, COLOR_VIBRA_PROMPT);
    console.print("\"\n");
    console.print("    Modular monolithic kernel in Rust\n");
    console.print("\n");
    CmdResult::Ok
}