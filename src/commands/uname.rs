use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        // uname — показать всё
        console.print("Vibra ");
        console.print(crate::version::KERNEL_VERSION);
        console.print(" ");
        console.print(crate::version::KERNEL_CODENAME);
        console.print(" ");
        console.print(crate::version::ARCHITECTURE);
        console.print("\n");
    } else if args[0] == "-a" {
        // uname -a — показать всё подробно
        console.print("Vibra ");
        console.print(crate::version::KERNEL_VERSION);
        console.print(" ");
        console.print(crate::version::KERNEL_CODENAME);
        console.print(" vibra ");
        console.print(crate::version::ARCHITECTURE);
        console.print(" ");
        console.print(crate::version::OS_VERSION);
        console.print(" \"");
        console.print(crate::version::OS_CODENAME);
        console.print("\"\n");
    } else if args[0] == "-r" {
        console.print(crate::version::KERNEL_VERSION);
        console.print("\n");
    } else if args[0] == "-s" {
        console.print("Vibra\n");
    } else if args[0] == "-m" {
        console.print(crate::version::ARCHITECTURE);
        console.print("\n");
    } else if args[0] == "-o" {
        console.print("Vibra\n");
    } else {
        console.print("Usage: uname [-a|-r|-s|-m|-o]\n");
    }
    CmdResult::Ok
}
