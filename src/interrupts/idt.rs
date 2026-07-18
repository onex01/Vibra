use crate::println;
use core::arch::asm;

#[repr(C)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0, selector: 0, ist: 0,
            type_attr: 0, offset_mid: 0, offset_high: 0, zero: 0,
        }
    }
    
    fn set_handler(&mut self, handler: u64) {
        self.offset_low = handler as u16;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.selector = 0x08;
        self.ist = 0;
        self.type_attr = 0x8E;
        self.zero = 0;
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe {
        println!("[IDT] Setting up IDT...");
        
        // Только исключения, БЕЗ IRQ
        IDT[0].set_handler(isr_divide_by_zero as *const () as u64);
        IDT[6].set_handler(isr_invalid_opcode as *const () as u64);
        IDT[8].set_handler(isr_double_fault as *const () as u64);
        IDT[13].set_handler(isr_general_protection as *const () as u64);
        IDT[14].set_handler(isr_page_fault as *const () as u64);
        
        let idt_ptr = IdtPointer {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        
        asm!("lidt [{}]", in(reg) &idt_ptr, options(readonly, nostack));
        println!("[IDT] IDT loaded (exceptions only, IRQs disabled)");
    }
}

#[no_mangle]
extern "C" fn isr_divide_by_zero() {
    println!("\n!!! DIVIDE BY ZERO !!!");
    super::halt_loop();
}

#[no_mangle]
extern "C" fn isr_invalid_opcode() {
    println!("\n!!! INVALID OPCODE !!!");
    super::halt_loop();
}

#[no_mangle]
extern "C" fn isr_double_fault() {
    println!("\n!!! DOUBLE FAULT !!!");
    super::halt_loop();
}

#[no_mangle]
extern "C" fn isr_general_protection() {
    println!("\n!!! GENERAL PROTECTION !!!");
    super::halt_loop();
}

#[no_mangle]
extern "C" fn isr_page_fault() {
    println!("\n!!! PAGE FAULT !!!");
    super::halt_loop();
}

pub fn ticks() -> u64 { 0 }