use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let user = crate::users::current_user();
    console.print_colored("Changing password for user: ", COLOR_YELLOW);
    console.print(&user.username);
    console.print("\n");
    console.print_colored("Password: ", COLOR_YELLOW);
    console.print("(always accepted in demo mode)\n");
    console.print_colored("Password updated successfully\n", COLOR_GREEN);
    CmdResult::Ok
}
