use super::CmdResult;
use crate::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Rebooting...\n", crate::framebuffer::COLOR_YELLOW);
    // Через QEMU reset: запись в порт 0x64 (keyboard controller reset)
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x64u16, in("al") 0xFEu8, options(nostack, preserves_flags));
    }
    // Если reset не сработал — просто hlt
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
