use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print_colored("Shutting down Vibra OS...\n", COLOR_YELLOW);

    // Выключаем прерывания
    crate::interrupts::disable();

    // Сбрасываем PS/2 контроллер (выключает клавиатуру и мышь)
    unsafe {
        crate::interrupts::pic::mask_all();
    }

    // QEMU shutdown через ACPI (порт 0x604 на q35)
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x604u16, in("al") 0x2000u16 as u8, options(nostack, preserves_flags));
    }

    // Если ACPI не сработал — пробуем через порт 0xB000 (ISA debug)
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0xB000u16, in("al") 0x00u8, options(nostack, preserves_flags));
    }

    // Если ничего не помогло — halt
    console.print_colored("System halted.\n", COLOR_RED);
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
