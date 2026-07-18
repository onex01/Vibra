use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_GREEN};
use crate::fs::{self, FileType};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let count = fs::fs_count();
    console.print_colored("Total: ", COLOR_YELLOW);
    console.print_num(count);
    console.print(" entries\n\n");

    for entry in fs::list_entries() {
        match entry.metadata.file_type {
            FileType::Directory => {
                console.print_colored("[DIR] ", COLOR_GREEN);
                console.print(entry.name());
            }
            FileType::File => {
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