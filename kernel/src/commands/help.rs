use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_VIBRA_PROMPT, COLOR_VIBRA_FG};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Available commands:\n", COLOR_YELLOW);

    // Базовые команды ядра
    for cmd in super::COMMANDS {
        print_cmd(console, cmd.name, cmd.help);
    }

    // Дополнительные команды из vibra OS
    let extra = super::EXTRA_COMMANDS.lock();
    for cmd in extra.iter() {
        print_cmd(console, cmd.name, cmd.help);
    }

    CmdResult::Ok
}

fn print_cmd(console: &mut Console, name: &str, help: &str) {
    console.print_colored("  ", COLOR_VIBRA_PROMPT);
    console.print_colored(name, COLOR_VIBRA_PROMPT);
    let pad = 12usize.saturating_sub(name.len());
    for _ in 0..pad { console.print(" "); }
    console.print_colored("- ", COLOR_VIBRA_FG);
    console.print(help);
    console.put_char('\n');
}
