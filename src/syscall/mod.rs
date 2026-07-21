// syscall/sysret — переход ring 3 → ring 0 и обратно.
//
// SYSCALL не сохраняет user RSP. Решение: сохраняем user RSP в static PERCPU
// перед sysretq (в tick_and_switch). На syscall entry читаем его обратно.

use crate::println;
use core::arch::asm;

const MSR_EFER: u32 = 0xC0000080;
const MSR_STAR: u32 = 0xC0000081;
const MSR_LSTAR: u32 = 0xC0000082;
const MSR_FMASK: u32 = 0xC0000084;

const EFER_SCE: u64 = 1 << 0;

pub const SYS_WRITE: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_YIELD: u64 = 2;

/// Per-Cpu data: [0]=user_rsp, [8]=kernel_rsp.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PerCpu {
    pub user_rsp: u64,
    pub kernel_rsp: u64,
}

pub static mut PERCPU: PerCpu = PerCpu { user_rsp: 0, kernel_rsp: 0 };

#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi);
    ((hi as u64) << 32) | (lo as u64)
}

#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    asm!("wrmsr", in("ecx") msr, in("eax") val as u32, in("edx") (val >> 32) as u32);
}

/// Naked stub: syscall entry из ring 3.
/// SYSCALL: RCX=user RIP, R11=user RFLAGS, RSP=TSS.rsp0 (kernel stack).
#[unsafe(naked)]
unsafe extern "sysv64" fn syscall_entry() -> ! {
    core::arch::naked_asm!(
        "swapgs",

        // Читаем user RSP из PERCPU
        "lea r10, [rip + {percpu}]",
        "mov r10, [r10]",        // r10 = PERCPU.user_rsp

        // Читаем kernel RSP из PERCPU
        "lea r11, [rip + {percpu}]",
        "mov rsp, [r11 + 8]",    // rsp = PERCPU.kernel_rsp

        // Сохраняем user context на kernel stack
        "push r11",              // placeholder (user RFLAGS будет восстановлен)
        "push rcx",              // user RIP
        "push r10",              // user RSP
        "push rdi",
        "push rsi",
        "push rdx",

        // Вызываем dispatcher
        "mov rdi, rax",          // rdi = syscall number
        "mov rsi, rsp",          // rsi = args pointer
        "call {dispatch}",

        // rax = return value

        // Восстанавливаем аргументы
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop r10",              // user RSP

        // Восстанавливаем user context
        "pop rcx",              // user RIP
        "pop r11",              // placeholder

        // Сохраняем user RSP обратно в PERCPU
        "lea r11, [rip + {percpu}]",
        "mov [r11], r10",       // PERCPU.user_rsp = user RSP

        // Восстанавливаем user RSP
        "mov rsp, r10",

        "swapgs",
        "sysretq",

        percpu = sym PERCPU,
        dispatch = sym syscall_dispatch,
    );
}

/// Dispatcher
#[no_mangle]
unsafe extern "sysv64" fn syscall_dispatch(sysnum: u64, args_ptr: *const u64) -> u64 {
    let rdi = core::ptr::read_volatile(args_ptr.add(0));
    let rsi = core::ptr::read_volatile(args_ptr.add(1));
    let rdx = core::ptr::read_volatile(args_ptr.add(2));

    match sysnum {
        SYS_WRITE => sys_write(rdi as usize, rsi as *const u8, rdx as usize),
        SYS_EXIT => {
            let pid = crate::task::current_task_id().unwrap_or(0);
            crate::task::exit_task(pid);
            crate::task::yield_now();
            0
        }
        SYS_YIELD => {
            crate::task::yield_now();
            0
        }
        _ => {
            println!("[SYSCALL] Unknown syscall: {}", sysnum);
            !0u64
        }
    }
}

/// sys_write(fd, ptr, len) — печатает строку в serial
unsafe fn sys_write(_fd: usize, ptr: *const u8, len: usize) -> u64 {
    if ptr as u64 >= 0xFFFF8000_0000_0000 {
        return !0u64;
    }
    if len > 4096 {
        return !0u64;
    }

    let slice = core::slice::from_raw_parts(ptr, len);
    for &b in slice {
        crate::serial::write_byte(b);
    }
    len as u64
}

/// Инициализация MSR для syscall/sysret
pub fn init() {
    println!("[SYSCALL] Initializing syscall/sysret...");

    unsafe {
        let mut efer = rdmsr(MSR_EFER);
        if efer & EFER_SCE == 0 {
            efer |= EFER_SCE;
            wrmsr(MSR_EFER, efer);
        }

        let star = (0x13u64 << 48) | (0x08u64 << 32);
        wrmsr(MSR_STAR, star);

        wrmsr(MSR_LSTAR, syscall_entry as u64);

        let fmask = (1u64 << 9) | (1u64 << 8) | (1u64 << 3);
        wrmsr(MSR_FMASK, fmask);

        let kstack_top = crate::task::get_kstack_top().unwrap_or(0);
        PERCPU.kernel_rsp = kstack_top;
        PERCPU.user_rsp = 0;

        println!("[SYSCALL] syscall/sysret ready (LSTAR={:#x})", syscall_entry as u64);
    }
}

/// Сохранить user RSP в PerCpu (вызывается перед возвратом в user space)
pub fn save_user_rsp(rsp: u64) {
    unsafe { PERCPU.user_rsp = rsp; }
}

/// Обновить kernel stack (вызывается при переключении задач)
pub fn update_kernel_stack(new_top: u64) {
    unsafe { PERCPU.kernel_rsp = new_top; }
}
