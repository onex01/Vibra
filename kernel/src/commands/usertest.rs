use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("Launching user-space process (ring 3)...\n");
    console.print("  syscall write(1, 'Hello from ring 3!\\n', 21)\n");
    console.print("  syscall exit(0)\n\n");

    crate::task::user::spawn_user_process("hello_world", crate::task::user::HELLO_USER);

    CmdResult::Ok
}
