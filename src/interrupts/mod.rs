pub mod idt;
pub mod pic;

use crate::println;

pub fn init() {
    println!("[INTR] Initializing interrupt subsystem...");
    pic::init();
    idt::init();
    println!("[INTR] Interrupts enabled!");
}

#[inline]
pub fn enable() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
}

#[inline]
pub fn disable() {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
}

#[inline]
pub fn wait() {
    unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
}

pub fn halt_loop() -> ! {
    loop { wait(); }
}