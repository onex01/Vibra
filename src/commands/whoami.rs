use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let user = crate::users::current_user();
    console.print(&user.username);
    console.put_char('\n');
    CmdResult::Ok
}
