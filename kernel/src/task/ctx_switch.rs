// Context Switch — naked-стабы и context switch primitives.
//
// Стековый фрейм (единый для interrupt и new task):
//   rsp+152: SS
//   rsp+144: RSP
//   rsp+136: RFLAGS
//   rsp+128: CS
//   rsp+120: RIP
//   rsp+112: RAX
//   rsp+104: RBX
//   rsp+96:  RCX
//   rsp+88:  RDX
//   rsp+80:  RSI
//   rsp+72:  RDI
//   rsp+64:  RBP
//   rsp+56:  R8
//   rsp+48:  R9
//   rsp+40:  R10
//   rsp+32:  R11
//   rsp+24:  R12
//   rsp+16:  R13
//   rsp+8:   R14
//   rsp+0:   R15
//
// CPU при IRQ пушит SS RSP RFLAGS CS RIP (5 слов снизу).
// Naked-stab пушит 15 GP-регистров сверху.
// Итого: 20 u64 = 160 байт.

use core::arch::naked_asm;

/// Размер контекста: 20 u64 = 160 байт
pub const CONTEXT_SIZE: usize = 20 * 8;

/// Naked-стаб для вектора 32 (PIT timer) и soft IRQ.
/// Вход: CPU кладёт SS, RSP, RFLAGS, CS, RIP.
/// Мы: сохраняем 15 GP, вызываем обработчик, загружаем новый RSP, восстанавливаем, iretq.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn timer_naked_stub() -> ! {
    naked_asm!(
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        "mov rdi, rsp",
        "call {handler}",

        "cmp rax, rdi",
        "je 2f",
        "mov rsp, rax",
        "2:",

        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",

        "iretq",

        handler = sym crate::task::tick_and_switch,
        
    );
}

/// Naked-стаб для soft IRQ (vector 0x81) — yield.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn softirq_naked_stub() -> ! {
    naked_asm!(
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        "mov rdi, rsp",
        "call {handler}",

        "cmp rax, rdi",
        "je 2f",
        "mov rsp, rax",
        "2:",

        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",

        "iretq",

        handler = sym crate::task::softirq_handler,
        
    );
}

/// Подготовить стек для НОВОЙ задачи (первый запуск).
/// Стек растёт ВНИЗ: stack_top — верхушка (наибольший адрес).
/// Кладём контекст НИЖЕ stack_top:
///
///   stack_top - 8:   SS
///   stack_top - 16:  RSP
///   stack_top - 24:  RFLAGS
///   stack_top - 32:  CS
///   stack_top - 40:  RIP        ← iretq frame (5 слов, CPU читает при iretq)
///   stack_top - 48:  RAX
///   stack_top - 56:  RBX
///   stack_top - 64:  RCX
///   stack_top - 72:  RDX
///   stack_top - 80:  RSI
///   stack_top - 88:  RDI
///   stack_top - 96:  RBP
///   stack_top - 104: R8
///   stack_top - 112: R9
///   stack_top - 120: R10
///   stack_top - 128: R11
///   stack_top - 136: R12
///   stack_top - 144: R13
///   stack_top - 152: R14
///   stack_top - 160: R15        ← bottom of context (saved_rsp указывает сюда)
///
/// При context switch: naked stub push'ит rax..r15 сверху вниз,
/// rsp оказывается на r15 — совпадает с форматом.
pub unsafe fn prepare_task_stack(stack_top: u64, entry: u64) -> u64 {
    // Базовый указатель — нижняя часть контекста
    let base = (stack_top - 160) as *mut u64;

    // 15 GP-регистров (снизу вверх: r15, r14, ..., rax)
    // Naked stub push order: rax first, rax last → в памяти rax выше r15
    // push rax → rsp-=8; push rbx → rsp-=8; ... push r15 → rsp-=8
    // В памяти (от низкого к высокому): r15 r14 ... rbx rax
    core::ptr::write_volatile(base.add(0), 0u64);    // r15
    core::ptr::write_volatile(base.add(1), 0u64);    // r14
    core::ptr::write_volatile(base.add(2), 0u64);    // r13
    core::ptr::write_volatile(base.add(3), 0u64);    // r12
    core::ptr::write_volatile(base.add(4), 0u64);    // r11
    core::ptr::write_volatile(base.add(5), 0u64);    // r10
    core::ptr::write_volatile(base.add(6), 0u64);    // r9
    core::ptr::write_volatile(base.add(7), 0u64);    // r8
    core::ptr::write_volatile(base.add(8), 0u64);    // rbp
    core::ptr::write_volatile(base.add(9), 0u64);    // rdi
    core::ptr::write_volatile(base.add(10), 0u64);   // rsi
    core::ptr::write_volatile(base.add(11), 0u64);   // rdx
    core::ptr::write_volatile(base.add(12), 0u64);   // rcx
    core::ptr::write_volatile(base.add(13), 0u64);   // rbx
    core::ptr::write_volatile(base.add(14), 0u64);   // rax

    // 2. iretq frame (5 слов над GP регистрами)
    core::ptr::write_volatile(base.add(15), entry);          // RIP
    core::ptr::write_volatile(base.add(16), 0x08u64);       // CS (kernel code)
    core::ptr::write_volatile(base.add(17), 0x202u64);      // RFLAGS (IF=1)
    core::ptr::write_volatile(base.add(18), stack_top);      // RSP
    core::ptr::write_volatile(base.add(19), 0x10u64);       // SS (kernel data)

    // saved_rsp = адрес r15 (нижний край контекста)
    stack_top - 160
}

/// Подготовить контекст для USER задачи (ring 3).
/// Аналогичен prepare_task_stack, но CS/SS = user сегменты.
/// user_rsp = адрес стека в user space для iretq frame.
pub unsafe fn prepare_user_task_stack(stack_top: u64, entry: u64, user_rsp: u64) -> u64 {
    let base = (stack_top - 160) as *mut u64;

    // Все 15 GP регистров = 0
    for i in 0..15 {
        core::ptr::write_volatile(base.add(i), 0u64);
    }

    // iretq frame: CS=0x23 (USER_CS), SS=0x1B (USER_DS)
    core::ptr::write_volatile(base.add(15), entry);          // RIP = code address
    core::ptr::write_volatile(base.add(16), 0x23u64);       // CS = USER_CS (ring 3)
    core::ptr::write_volatile(base.add(17), 0x202u64);      // RFLAGS (IF=1)
    core::ptr::write_volatile(base.add(18), user_rsp);       // RSP = user stack
    core::ptr::write_volatile(base.add(19), 0x1Bu64);       // SS = USER_DS (ring 3)

    stack_top - 160
}
