use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let username = args.first().unwrap_or(&"root");

    match crate::users::switch_user(username) {
        Ok(()) => {
            let user = crate::users::current_user();
            console.print_colored("Switched to user: ", COLOR_GREEN);
            console.print(&user.username);
            console.print(" (uid=");
            console.print_num(user.uid as usize);
            console.print(")\n");
        }
        Err(e) => {
            console.print_colored("su: ", COLOR_RED);
            console.print(e);
            console.put_char('\n');
        }
    }
    CmdResult::Ok
}
