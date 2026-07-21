// x86_64 architecture-specific code.
//
// Модули для x86_64 платформы:
// - interrupts (IDT, PIC, APIC)
// - memory (paging, VMM, PMM, heap)
// - syscall (syscall/sysret)
// - task (context switch naked stubs)
// - PCI/AHCI drivers
// - PS/2 keyboard
// - COM1 serial

pub mod interrupts {
    pub use crate::interrupts::*;
}

pub mod memory {
    pub use crate::memory::*;
}

pub mod task {
    pub use crate::task::ctx_switch;
}

/// Инициализация x86-specific модулей
pub fn init() {
    // Пока заглушка — полная инициализация через main.rs
}
