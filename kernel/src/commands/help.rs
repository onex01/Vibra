use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_VIBRA_PROMPT, COLOR_VIBRA_FG};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    const NAME_W: usize = 9;
    const HELP_W: usize = 24;

    console.put_char('\n');

    // Верхняя граница: ╔═══...═══╗
    console.put_char(0xC9 as char); // ╔
    for _ in 0..(NAME_W + HELP_W + 2) {
        console.put_char(0xCD as char); // ═
    }
    console.put_char(0xBB as char); // ╗
    console.put_char('\n');

    // Заголовок: ║    Available commands    ║
    console.put_char(0xBA as char); // ║
    console.print_colored("  Available commands", COLOR_YELLOW);
    let remaining = NAME_W + HELP_W + 2 - 21;
    for _ in 0..remaining {
        console.print(" ");
    }
    console.put_char(0xBA as char); // ║
    console.put_char('\n');

    // Разделитель: ╠═══...═══╣
    console.put_char(0xCC as char); // ╠
    for _ in 0..(NAME_W + HELP_W + 2) {
        console.put_char(0xCD as char); // ═
    }
    console.put_char(0xB9 as char); // ╣
    console.put_char('\n');

    // Базовые команды
    for cmd in super::COMMANDS {
        print_cmd_table(console, cmd.name, cmd.help, NAME_W, HELP_W);
    }

    // Дополнительные команды
    let extra = super::EXTRA_COMMANDS.lock();
    for cmd in extra.iter() {
        print_cmd_table(console, cmd.name, cmd.help, NAME_W, HELP_W);
    }

    // Нижняя граница: ╚═══...═══╝
    console.put_char(0xC8 as char); // ╚
    for _ in 0..(NAME_W + HELP_W + 2) {
        console.put_char(0xCD as char); // ═
    }
    console.put_char(0xBC as char); // ╝
    console.put_char('\n');

    CmdResult::Ok
}

fn print_cmd_table(console: &mut Console, name: &str, help: &str, name_w: usize, help_w: usize) {
    // ║ name    ║ help text                    ║
    console.put_char(0xBA as char); // ║
    console.print(" ");
    console.print_colored(name, COLOR_VIBRA_PROMPT);
    let pad = name_w.saturating_sub(name.len());
    for _ in 0..pad { console.print(" "); }
    console.print(" ");
    console.put_char(0xBA as char); // ║
    console.print(" ");
    console.print_colored(help, COLOR_VIBRA_FG);
    let help_pad = help_w.saturating_sub(help.len());
    for _ in 0..help_pad { console.print(" "); }
    console.print(" ");
    console.put_char(0xBA as char); // ║
    console.put_char('\n');
}
