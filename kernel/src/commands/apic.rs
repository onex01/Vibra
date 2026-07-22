use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let active = crate::interrupts::apic::is_active();
    let has_apic = crate::interrupts::apic::has_apic();

    console.print("APIC Status:\n");
    console.print("  Detected: ");
    console.print(if has_apic { "yes" } else { "no" });
    console.print("\n");

    console.print("  Active:   ");
    console.print(if active { "yes (full APIC, PIC offline)" } else { "no (PIC primary)" });
    console.print("\n");

    if active {
        console.print("  Timer:    LAPIC periodic 100Hz, vector 32\n");
        console.print("  Keyboard: IO APIC GSI1 -> vector 33\n");
        console.print("  Serial:   polling (no IO APIC redirect)\n");
    }

    CmdResult::Ok
}
