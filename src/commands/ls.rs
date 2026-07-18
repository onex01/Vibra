use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN};
use crate::fs;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let current_dir = fs::get_current_dir();
    
    console.print_colored("Directory: ", COLOR_YELLOW);
    console.print(current_dir);
    console.print("\n\n");

    let count = fs::fs_count();
    console.print_colored("Total: ", COLOR_YELLOW);
    console.print_num(count);
    console.print(" entries\n\n");

    for entry in fs::list_dir(current_dir) {
        match entry.metadata.file_type {
            fs::FileType::Directory => {
                console.print_colored("[DIR] ", COLOR_GREEN);
                console.print(entry.name());
            }
            fs::FileType::File => {
                console.print("      ");
                console.print(entry.name());
                console.print(" (");
                console.print_num(entry.metadata.size);
                console.print(" bytes)");
            }
        }
        console.put_char('\n');
    }
    CmdResult::Ok
}