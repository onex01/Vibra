// syscall/sysret — переход ring 3 → ring 0 и обратно.

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

/// Per-Cpu data: [0]=user_rsp, [8]=kernel_rsp, [16]=user_rflags.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PerCpu {
    pub user_rsp: u64,
    pub kernel_rsp: u64,
    pub user_rflags: u64,
}

pub static mut PERCPU: PerCpu = PerCpu { user_rsp: 0, kernel_rsp: 0, user_rflags: 0 };

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
#[unsafe(naked)]
unsafe extern "sysv64" fn syscall_entry() -> ! {
    core::arch::naked_asm!(
        "swapgs",

        // Читаем user RSP и RFLAGS из PERCPU
        "lea r10, [rip + {percpu}]",
        "mov r10, [r10]",        // r10 = PERCPU.user_rsp
        "mov r11, [r10 + 16 - 16]", // не можем — используем другой подход

        // Читаем kernel RSP
        "lea r11, [rip + {percpu}]",
        "mov rsp, [r11 + 8]",    // rsp = PERCPU.kernel_rsp

        // Сохраняем user context: rdi, rsi, rdx (args) + rcx (RIP) + r11 (RFLAGS)
        // Стек: [rsp+0]=rdi, [+8]=rsi, [+16]=rdx, [+24]=rcx, [+32]=r11
        "push r11",              // placeholder
        "push rcx",              // user RIP
        "push rdx",
        "push rsi",
        "push rdi",

        // Вызываем dispatcher
        "mov rdi, rax",          // rdi = syscall number
        "mov rsi, rsp",          // rsi = args pointer
        "call {dispatch}",

        // rax = return value — сохраняем в r10
        "mov r10, rax",

        // Восстанавливаем аргументы
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",              // user RIP
        "pop r11",              // placeholder

        // Восстанавливаем user RSP из PERCPU
        "lea rax, [rip + {percpu}]",
        "mov rax, [rax]",       // rax = PERCPU.user_rsp
        "mov rsp, rax",

        // Восстанавливаем user RFLAGS из PERCPU
        "lea rax, [rip + {percpu}]",
        "mov r11, [rax + 16]",  // r11 = PERCPU.user_rflags

        // rax = return value (был в r10)
        "mov rax, r10",

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

    crate::println!("[SYSCALL] num={} rdi={} rsi={:#x} rdx={}", sysnum, rdi, rsi, rdx);

    match sysnum {
        SYS_WRITE => sys_write(rdi as usize, rsi as *const u8, rdx as usize),
        SYS_EXIT => {
            let pid = crate::task::current_task_id().unwrap_or(0);
            crate::task::exit_task(pid);
            println!("[SYSCALL] Process {} exited", pid);
            // Возвращаем 0 — sysretq вернётся в user space,
            // но задача помечена как Zombie и не будет выбрана планировщиком.
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

/// sys_write(fd, ptr, len)
unsafe fn sys_write(_fd: usize, ptr: *const u8, len: usize) -> u64 {
    if ptr as u64 >= 0xFFFF8000_0000_0000 { return !0u64; }
    if len > 4096 { return !0u64; }
    let slice = core::slice::from_raw_parts(ptr, len);
    for &b in slice { crate::serial::write_byte(b); }
    len as u64
}

pub fn init() {
    println!("[SYSCALL] Initializing syscall/sysret...");
    unsafe {
        let mut efer = rdmsr(MSR_EFER);
        if efer & EFER_SCE == 0 { efer |= EFER_SCE; wrmsr(MSR_EFER, efer); }
        wrmsr(MSR_STAR, (0x13u64 << 48) | (0x08u64 << 32));
        wrmsr(MSR_LSTAR, syscall_entry as u64);
        wrmsr(MSR_FMASK, (1u64 << 9) | (1u64 << 8) | (1u64 << 3));
        let kstack_top = crate::task::get_kstack_top().unwrap_or(0);
        PERCPU.kernel_rsp = kstack_top;
        PERCPU.user_rsp = 0;
        PERCPU.user_rflags = 0x200; // IF=1
        println!("[SYSCALL] PERCPU: kernel_rsp={:#x}, user_rsp={:#x}", kstack_top, 0u64);
        println!("[SYSCALL] syscall/sysret ready (LSTAR={:#x})", syscall_entry as u64);
    }
}

pub fn save_user_rsp(rsp: u64) {
    unsafe { PERCPU.user_rsp = rsp; }
}

pub fn save_user_rflags(rflags: u64) {
    unsafe { PERCPU.user_rflags = rflags; }
}

pub fn update_kernel_stack(new_top: u64) {
    unsafe { PERCPU.kernel_rsp = new_top; }
}
