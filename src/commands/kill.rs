use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.print_colored("Usage: kill <PID>\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    let pid: u32 = match args[0].parse() {
        Ok(p) => p,
        Err(_) => {
            console.print_colored("Invalid PID\n", COLOR_RED);
            return CmdResult::Ok;
        }
    };

    if pid == 0 {
        console.print_colored("Cannot kill kernel process (kshell)\n", COLOR_RED);
        return CmdResult::Ok;
    }

    crate::task::exit_task(pid);
    console.print("Process ");
    console.print_num(pid as usize);
    console.print(" killed\n");

    CmdResult::Ok
}
