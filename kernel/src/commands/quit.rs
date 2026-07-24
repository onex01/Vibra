use super::CmdResult;
use crate::framebuffer::Console;
use crate::println;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("System halted.\n");

    // ACPI shutdown через порт 0x604 (q35)
    unsafe {
        core::arch::asm!("out dx, ax", in("dx") 0x604u16, in("ax") 0x2000u16, options(nostack, preserves_flags));
    }

    // Альтернативный ACPI shutdown через порт 0x92
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x92u16, in("al") 0x03u8, options(nostack, preserves_flags));
    }

    println!("System halted.");
    loop {
        crate::interrupts::halt();
    }
}
