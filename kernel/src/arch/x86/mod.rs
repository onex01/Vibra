// x86_64 architecture-specific code.
//
// Этот модуль содержит x86-специфичные компоненты.
// В будущем все x86 модули будут перемещены сюда из src/.

/// Инициализация x86-specific модулей (вызывается из main.rs)
pub fn init() {
    // Пока заглушка — основная инициализация через main.rs
    // Постепенно будем перемещать сюда: gdt, idt, interrupts, paging, vmm, pmm, heap
}

/// x86 specific constants
pub const PAGE_SIZE: u64 = 4096;
pub const HHDM_OFFSET_DEFAULT: u64 = 0xFFFF8000_0000_0000;
