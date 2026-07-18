use crate::println;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};
use super::pic;

// Частота системного таймера (PIT), Гц
pub const TIMER_HZ: u64 = 100;

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
        self.set_handler_with_ist(handler, 0);
    }

    // ist: 1-based индекс в TSS.IST (0 = обычный стек)
    fn set_handler_with_ist(&mut self, handler: u64, ist: u16) {
        // Берём ТЕКУЩИЙ селектор кода — IDT строится после gdt::init(),
        // так что это будет наш KERNEL_CS.
        let cs: u16;
        unsafe { asm!("mov {0:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags)); }
        self.offset_low = handler as u16;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.selector = cs;
        self.ist = (ist & 0x7) as u8;
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

static TICKS: AtomicU64 = AtomicU64::new(0);

#[inline]
unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

// Программируем PIT (канал 0) на TIMER_HZ
unsafe fn init_pit() {
    let divisor = (1_193_182u64 / TIMER_HZ) as u16;
    outb(0x43, 0x36); // канал 0, lobyte/hibyte, режим 3 (square wave)
    outb(0x40, divisor as u8);
    outb(0x40, (divisor >> 8) as u8);
}

pub fn init() {
    unsafe {
        println!("[IDT] Setting up IDT...");

        // Исключения CPU.
        // ВАЖНО: обработчики используют ABI "x86-interrupt" — компилятор сам
        // сохраняет регистры и завершает обработчик через iretq.
        IDT[0].set_handler(isr_divide_by_zero as *const () as u64);
        IDT[2].set_handler_with_ist(isr_nmi as *const () as u64, crate::gdt::NMI_IST_INDEX);
        IDT[6].set_handler(isr_invalid_opcode as *const () as u64);
        // Double Fault на отдельном IST-стеке: сработает даже если RSP битый
        IDT[8].set_handler_with_ist(isr_double_fault as *const () as u64, crate::gdt::DOUBLE_FAULT_IST_INDEX);
        IDT[13].set_handler(isr_general_protection as *const () as u64);
        IDT[14].set_handler(isr_page_fault as *const () as u64);

        // Аппаратные прерывания (после ремапа PIC: 32..47)
        IDT[pic::PIC1_OFFSET as usize + 0].set_handler(isr_timer as *const () as u64);     // IRQ0
        IDT[pic::PIC1_OFFSET as usize + 1].set_handler(isr_keyboard as *const () as u64);  // IRQ1
        IDT[pic::PIC1_OFFSET as usize + 7].set_handler(isr_spurious_master as *const () as u64); // IRQ7
        IDT[pic::PIC2_OFFSET as usize + 7].set_handler(isr_spurious_slave as *const () as u64);  // IRQ15

        let idt_ptr = IdtPointer {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };

        asm!("lidt [{}]", in(reg) &idt_ptr, options(readonly, nostack));

        init_pit();
        println!("[IDT] IDT loaded (exceptions + IRQ0 timer @ {} Hz + IRQ1 keyboard)", TIMER_HZ);
    }
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// ============ Аппаратные прерывания ============

extern "x86-interrupt" fn isr_timer(_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe { pic::eoi(0); }
}

extern "x86-interrupt" fn isr_keyboard(_frame: InterruptStackFrame) {
    // Скан-код нужно прочитать обязательно, иначе контроллер
    // не выдаст следующее прерывание
    let scancode = unsafe { inb(0x60) };
    crate::keyboard::handle_interrupt(scancode);
    unsafe { pic::eoi(1); }
}

// Ложное (spurious) IRQ7: EOI отправлять НЕ нужно
extern "x86-interrupt" fn isr_spurious_master(_frame: InterruptStackFrame) {}

// Ложное IRQ15: EOI только мастеру (он видел каскад IRQ2)
extern "x86-interrupt" fn isr_spurious_slave(_frame: InterruptStackFrame) {
    unsafe { pic::eoi(0); }
}

// ============ Исключения CPU ============

fn dump_frame(frame: &InterruptStackFrame) {
    let rip = frame.instruction_pointer;
    let cs = frame.code_segment;
    let flags = frame.cpu_flags;
    let rsp = frame.stack_pointer;
    println!("  RIP={:#018x} CS={:#x} RFLAGS={:#x} RSP={:#018x}", rip, cs, flags, rsp);
}

extern "x86-interrupt" fn isr_divide_by_zero(frame: InterruptStackFrame) {
    println!("\n!!! DIVIDE BY ZERO !!!");
    dump_frame(&frame);
    super::halt_loop();
}

// NMI: на своём IST-стеке, просто логируем и продолжаем
extern "x86-interrupt" fn isr_nmi(frame: InterruptStackFrame) {
    println!("\n[NMI] Non-maskable interrupt received");
    dump_frame(&frame);
}

extern "x86-interrupt" fn isr_invalid_opcode(frame: InterruptStackFrame) {
    println!("\n!!! INVALID OPCODE !!!");
    dump_frame(&frame);
    super::halt_loop();
}

// Исключения 8, 13, 14 кладут на стек код ошибки — сигнатура обязана его принимать!
extern "x86-interrupt" fn isr_double_fault(frame: InterruptStackFrame, error_code: u64) -> ! {
    println!("\n!!! DOUBLE FAULT (error={:#x}) !!!", error_code);
    dump_frame(&frame);
    super::halt_loop();
}

extern "x86-interrupt" fn isr_general_protection(frame: InterruptStackFrame, error_code: u64) {
    println!("\n!!! GENERAL PROTECTION (error={:#x}) !!!", error_code);
    dump_frame(&frame);
    super::halt_loop();
}

extern "x86-interrupt" fn isr_page_fault(frame: InterruptStackFrame, error_code: u64) {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)); }
    println!("\n!!! PAGE FAULT !!!");
    println!("  Accessed address (CR2) = {:#018x}", cr2);
    println!("  Error code = {:#x} [{} | {} | {}]",
        error_code,
        if error_code & 1 != 0 { "protection" } else { "not-present" },
        if error_code & 2 != 0 { "write" } else { "read" },
        if error_code & 4 != 0 { "user" } else { "kernel" },
    );
    dump_frame(&frame);
    super::halt_loop();
}
