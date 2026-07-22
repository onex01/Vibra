use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 { console.print(" "); }
        console.print(arg);
    }
    console.put_char('\n');
    CmdResult::Ok
}