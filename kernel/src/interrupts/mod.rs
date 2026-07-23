pub mod idt;
pub mod pic;
pub mod apic;
pub mod msi;

use crate::println;

pub fn init() {
    println!("[INTR] Initializing interrupt subsystem...");
    pic::init();
    idt::init();
    msi::init();
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

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let flags: u64;
    unsafe { core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags)); }
    let were_enabled = flags & (1 << 9) != 0;
    if were_enabled { disable(); }
    let result = f();
    if were_enabled { enable(); }
    result
}

pub fn halt_loop() -> ! {
    loop { wait(); }
}
