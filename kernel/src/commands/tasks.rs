use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_WHITE};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let tasks = crate::task::list_tasks();
    let (ticks, ctx_sw, count) = crate::task::stats();

    console.print_colored("Task Manager\n", COLOR_CYAN);
    console.print_colored("═══════════════════════════════════════\n", COLOR_CYAN);
    console.print_colored("  PID  STATE     TIME    NAME\n", COLOR_YELLOW);
    console.print_colored("────── ───────── ─────── ──────────────\n", COLOR_CYAN);

    for (id, name, state) in &tasks {
        if *id < 10 {
            console.print("  ");
        } else if *id < 100 {
            console.print(" ");
        }
        console.print_num(*id as usize);
        console.print("  ");

        match *state {
            "Running" => console.print_colored(state, COLOR_GREEN),
            "Ready" => console.print_colored(state, COLOR_YELLOW),
            _ => console.print_colored(state, COLOR_WHITE),
        }

        let padding = 8 - state.len();
        for _ in 0..padding { console.put_char(' '); }

        console.print(name);
        console.put_char('\n');
    }

    console.put_char('\n');
    console.print("  Tasks: ");
    console.print_num(count);
    console.print("  Ticks: ");
    console.print_num(ticks as usize);
    console.print("  Switches: ");
    console.print_num(ctx_sw as usize);
    console.put_char('\n');

    CmdResult::Ok
}
