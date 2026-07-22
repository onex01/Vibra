// User-space процесс: загрузка байтов кода в user pages и запуск через iretq.
//
// Используем map_user_page_fresh — создаёт новые page tables в свободном PML4 slot,
// избегая Limine 2MB huge pages.

use crate::println;
use crate::memory::vmm;
use crate::memory::paging;
use core::sync::atomic::Ordering;

const PAGE_SIZE: u64 = 4096;

/// User-код: write "Hello from ring 3!\n", затем infinite loop
/// Layout:
///   0-6:   mov rax, 0      (7 bytes)
///   7-13:  mov rdi, 1      (7 bytes)
///   14-20: lea rsi, [rip+X] (7 bytes, RIP after=21, X=string_offset-21)
///   21-27: mov rdx, 21     (7 bytes)
///   28-29: syscall          (2 bytes)
///   30-31: jmp $            (2 bytes)
///   32+:   "Hello from ring 3!\n" (21 bytes)
///   X = 32 - 21 = 11 = 0x0B
pub const HELLO_USER: &[u8] = &[
    0x48, 0xc7, 0xc0, 0x00, 0x00, 0x00, 0x00,  // mov rax, 0
    0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00,  // mov rdi, 1
    0x48, 0x8d, 0x35, 0x0B, 0x00, 0x00, 0x00,  // lea rsi, [rip+0x0B]
    0x48, 0xc7, 0xc2, 0x15, 0x00, 0x00, 0x00,  // mov rdx, 21
    0x0f, 0x05,                                   // syscall
    0xEB, 0xFE,                                   // jmp $
    b'H', b'e', b'l', b'l', b'o', b' ',
    b'f', b'r', b'o', b'm', b' ',
    b'r', b'i', b'n', b'g', b' ',
    b'3', b'!', b'\n',
];

/// Запустить user-процесс с указанными байтами кода.
pub fn spawn_user_process(name: &str, code: &[u8]) {
    let hhdm = paging::HHDM_OFFSET.load(Ordering::Relaxed);

    // 1. Физическая страница для кода
    let code_phys = match crate::memory::pmm::alloc_frame() {
        Some(f) => f as u64,
        None => { println!("[USER] ERROR: no free frame for code"); return; }
    };

    // Копируем код
    let code_page = hhdm + code_phys;
    let copy_len = code.len().min(PAGE_SIZE as usize);
    unsafe {
        core::ptr::write_bytes(code_page as *mut u8, 0, PAGE_SIZE as usize);
        core::ptr::copy_nonoverlapping(code.as_ptr(), code_page as *mut u8, copy_len);
    }

    // Маппим в свежий PML4 slot (не конфликтует с Limine)
    let user_code = match vmm::map_user_page_fresh(code_phys, false, true) {
        Some(v) => v,
        None => { println!("[USER] ERROR: failed to map code page"); return; }
    };

    // 2. Физическая страница для стека
    let stack_phys = match crate::memory::pmm::alloc_frame() {
        Some(f) => f as u64,
        None => { println!("[USER] ERROR: no free frame for stack"); return; }
    };
    let user_stack_top = user_code + PAGE_SIZE;

    let stack_page = hhdm + stack_phys;
    unsafe { core::ptr::write_bytes(stack_page as *mut u8, 0, PAGE_SIZE as usize); }

    // Стек маппим в тот же PML4 slot (identity: stack_phys → stack_phys)
    // Но проще: маппим через map_user_page_fresh в другой slot
    let user_stack_base = match vmm::map_user_page_fresh(stack_phys, true, false) {
        Some(v) => v,
        None => { println!("[USER] ERROR: failed to map stack page"); return; }
    };

    // Используем user_stack_base + PAGE_SIZE как верх стека
    let stack_top = user_stack_base + PAGE_SIZE;

    println!("[USER] Code: virt={:#x} → phys={:#x}", user_code, code_phys);
    println!("[USER] Stack: virt={:#x} → phys={:#x}", user_stack_base, stack_phys);

    // 3. Kernel stack
    let (kstack_ptr, kstack_top) = match super::alloc_kstack() {
        Some(v) => v,
        None => { println!("[USER] ERROR: no kernel stack"); return; }
    };

    let ctx_rsp = unsafe {
        crate::task::ctx_switch::prepare_user_task_stack(kstack_top, user_code, stack_top)
    };

    // 4. Регистрируем
    let mut guard = super::SCHEDULER.lock();
    if let Some(ref mut sched) = *guard {
        let id = sched.next_id;
        sched.next_id += 1;
        sched.tasks.push(super::Task {
            id,
            name: String::from(name),
            state: super::TaskState::Ready,
            priority: super::Priority::Normal,
            time_slices: 0,
            wake_time: None,
            entry: None,
            saved_rsp: ctx_rsp,
            kstack_ptr,
            kstack_layout: core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap(),
            kstack_top: Some(kstack_top),
            user_rsp: stack_top,
            user_rflags: 0x200, // IF=1
        });
        println!("[USER] User process '{}' spawned (PID={})", name, id);
    }
}

use super::KERNEL_STACK_SIZE;
use alloc::string::String;
