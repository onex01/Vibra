use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let user = crate::users::current_user();
    console.print("uid=");
    console.print_num(user.uid as usize);
    console.print("(");
    console.print(&user.username);
    console.print(") gid=");
    console.print_num(user.gid as usize);
    console.print("(users)");
    console.put_char('\n');
    CmdResult::Ok
}
