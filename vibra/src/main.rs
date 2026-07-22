// Vibra OS — простой загрузчик ядра.
//
// Этот crate зависит от vibra-kernel (библиотеки).
// Он просто вызывает boot ядра.

#![no_std]
#![no_main]

extern crate alloc;
extern crate vibra_kernel;

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vibra_kernel::boot();
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[PANIC] {}", info);
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}
