use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_RED};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.print_colored("Vibra Script (.vs)\n", COLOR_YELLOW);
        console.print_colored("Usage: run <file.vs> | run -e code\n", COLOR_YELLOW);
        console.print_colored("Commands: var, print, if/else, while, beep, sleep, exit\n", COLOR_YELLOW);
        console.print_colored("Operators: + - * / % == != < > <= >=\n\n", COLOR_YELLOW);
        console.print("  run -e \"print 2 + 3\"\n");
        console.print("  run -e \"var x = 0; while x < 5 { print x; x = x + 1 }\"\n");
        return CmdResult::Ok;
    }

    if args[0] == "-e" || args[0] == "--exec" {
        // Join all remaining args with spaces
        let mut code = alloc::string::String::new();
        for (i, a) in args.iter().enumerate() {
            if i == 0 { continue; } // skip -e
            if !code.is_empty() { code.push(' '); }
            code.push_str(a);
        }
        if code.is_empty() {
            console.print_colored("Usage: run -e code\n", COLOR_RED);
        } else {
            match crate::script::run_script(&code, console) {
                crate::script::ScriptResult::Ok => {}
                crate::script::ScriptResult::Error(e) => {
                    console.print_colored("Error: ", COLOR_RED);
                    console.print(&e);
                    console.put_char('\n');
                }
                crate::script::ScriptResult::Exit => {}
            }
        }
        return CmdResult::Ok;
    }

    match crate::script::run_file(args[0], console) {
        crate::script::ScriptResult::Ok => {}
        crate::script::ScriptResult::Error(e) => {
            console.print_colored("Error: ", COLOR_RED);
            console.print(&e);
            console.put_char('\n');
        }
        crate::script::ScriptResult::Exit => {}
    }
    CmdResult::Ok
}
