use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_GREEN};
use crate::fs;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.put_char('\n');
        return CmdResult::Ok;
    }

    // Проверяем наличие > (redirect)
    let mut redirect_idx = None;
    for (i, arg) in args.iter().enumerate() {
        if *arg == ">" { redirect_idx = Some(i); break; }
    }

    if let Some(idx) = redirect_idx {
        // echo text > filename
        let text: alloc::string::String = args[..idx].iter().enumerate()
            .map(|(i, a)| if i > 0 { alloc::format!(" {}", a) } else { alloc::string::String::from(*a) })
            .collect();

        if idx + 1 >= args.len() {
            console.print_colored("Usage: echo text > filename\n", COLOR_RED);
            return CmdResult::Ok;
        }

        let filename = args[idx + 1];
        let data = text.as_bytes();
        console.print("Writing ");
        console.print_num(data.len());
        console.print(" bytes to '");
        console.print(filename);
        console.print("'\n");
        if let Err(e) = fs::write_file(filename, data) {
            console.print_colored("Error: ", COLOR_RED);
            console.print(&alloc::format!("{}", e));
            console.put_char('\n');
        }
    } else {
        // Обычный echo
        for (i, arg) in args.iter().enumerate() {
            if i > 0 { console.print(" "); }
            console.print(arg);
        }
        console.put_char('\n');
    }
    CmdResult::Ok
}
