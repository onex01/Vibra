use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_VIBRA_PROMPT};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("\n");
    console.print_colored("     __     ___ _           \n", COLOR_CYAN);
    console.print_colored("     \\ \\   / (_) |__  _ __ __ _ \n", COLOR_CYAN);
    console.print_colored("      \\ \\ / /| | '_ \\| '__/ _` |\n", COLOR_CYAN);
    console.print_colored("       \\ V / | | |_) | | | (_| |\n", COLOR_CYAN);
    console.print_colored("        \\_/  |_|_.__/|_|  \\__,_|\n", COLOR_CYAN);
    console.print("\n");
    console.print_colored("    Vibra OS v0.4 \"Photon\" | Kernel v0.4.0\n", COLOR_VIBRA_PROMPT);
    console.print("    Modular monolithic kernel in Rust\n");
    console.print("\n");
    CmdResult::Ok
}