use crate::framebuffer::Console;
use crate::commands::CmdResult;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    // Inject a test string via keyboard queue
    let test = "hid_manual\n";
    for &b in test.as_bytes() {
        let key = match b {
            b'\n' => crate::keyboard::Key::Enter,
            b' ' => crate::keyboard::Key::Char(' '),
            _ if (0x20..=0x7E).contains(&b) => crate::keyboard::Key::Char(b as char),
            _ => continue,
        };
        crate::keyboard::inject_key(key);
    }
    console.put_char('\n');
    console.print("[hidtest] injected\n");
    CmdResult::Ok
}
