use super::{CmdResult, COMMANDS};
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_VIBRA_PROMPT, COLOR_VIBRA_FG};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Available commands:\n", COLOR_YELLOW);
    for cmd in COMMANDS {
        console.print_colored("  ", COLOR_VIBRA_PROMPT);
        console.print_colored(cmd.name, COLOR_VIBRA_PROMPT);
        // Padding для выравнивания
        let pad = 10usize.saturating_sub(cmd.name.len());
        for _ in 0..pad { console.print(" "); }
        console.print_colored("- ", COLOR_VIBRA_FG);
        console.print(cmd.help);
        console.put_char('\n');
    }
    CmdResult::Ok
}