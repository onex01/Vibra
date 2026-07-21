// syscall/sysret — переход ring 3 → ring 0 и обратно.
//
// SYSCALL не сохраняет user RSP. Решение:
//   - Перед iretq (timer_naked_stub): сохраняем user RSP в PERCPU
//   - На syscall entry: читаем user RSP из PERCPU, переключаемся на kernel stack

use crate::println;
use core::arch::asm;

const MSR_EFER: u32 = 0xC0000080;
const MSR_STAR: u32 = 0xC0000081;
const MSR_LSTAR: u32 = 0xC0000082;
const MSR_FMASK: u32 = 0xC0000084;

pub const SYS_WRITE: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_YIELD: u64 = 2;

/// Per-Cpu: [0]=user_rsp, [8]=kernel_rsp, [16]=user_rflags
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PerCpu {
    pub user_rsp: u64,
    pub kernel_rsp: u64,
    pub user_rflags: u64,
}

pub static mut PERCPU: PerCpu = PerCpu { user_rsp: 0, kernel_rsp: 0, user_rflags: 0x200 };

/// Naked stub: syscall entry.
/// After SYSCALL: RCX=user RIP, R11=user RFLAGS, RSP=TSS.rsp0.
#[unsafe(naked)]
unsafe extern "sysv64" fn syscall_entry() -> ! {
    core::arch::naked_asm!(
        "swapgs",

        // rdi/rsi/rdx = user args (сохраняем на kernel stack позже)
        // rcx = user RIP, r11 = user RFLAGS
        // rax = syscall number

        // Читаем kernel RSP из PERCPU
        "lea r10, [rip + {percpu}]",
        "mov rsp, [r10 + 8]",    // rsp = PERCPU.kernel_rsp

        // Сохраняем user context на kernel stack
        "push r11",              // user RFLAGS
        "push rcx",              // user RIP
        "push rdx",              // arg2
        "push rsi",              // arg1
        "push rdi",              // arg0 → rsp+0

        // Вызываем dispatcher
        "mov rdi, rax",          // rdi = syscall number
        "mov rsi, rsp",          // rsi = args pointer
        "call {dispatch}",

        // rax = return value → сохраняем
        "mov r10, rax",

        // Восстанавливаем user context
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",              // user RIP
        "pop r11",              // user RFLAGS

        // Восстанавливаем user RSP из PERCPU
        "lea rax, [rip + {percpu}]",
        "mov rsp, [rax]",       // rsp = PERCPU.user_rsp

        // rax = return value
        "mov rax, r10",

        // rcx = user RIP, r11 = user RFLAGS — всё готово
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
            // Не возвращаемся — sysretq не произойдёт.
            // Планировщик не выберет Zombie задачу на следующем тике.
            loop {
                unsafe { asm!("sti; hlt", options(nomem, nostack)); }
            }
        }
        SYS_YIELD => {
            crate::task::yield_now();
            0
        }
        _ => {
            println!("[SYSCALL] Unknown: {}", sysnum);
            !0u64
        }
    }
}

/// sys_write(fd, ptr, len)
unsafe fn sys_write(_fd: usize, ptr: *const u8, len: usize) -> u64 {
    if ptr as u64 >= 0xFFFF8000_0000_0000 || len > 4096 {
        return !0u64;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    for &b in slice { crate::serial::write_byte(b); }
    len as u64
}

pub fn init() {
    println!("[SYSCALL] Initializing syscall/sysret...");
    unsafe {
        let mut efer = rdmsr(MSR_EFER);
        if efer & (1 << 0) == 0 { efer |= 1; wrmsr(MSR_EFER, efer); }
        wrmsr(MSR_STAR, (0x13u64 << 48) | (0x08u64 << 32));
        wrmsr(MSR_LSTAR, syscall_entry as u64);
        wrmsr(MSR_FMASK, (1 << 9) | (1 << 8) | (1 << 3));
        let k = crate::task::get_kstack_top().unwrap_or(0);
        PERCPU.kernel_rsp = k;
        PERCPU.user_rsp = 0;
        PERCPU.user_rflags = 0x200;
        println!("[SYSCALL] syscall/sysret ready");
    }
}

pub fn save_user_rsp(rsp: u64) { unsafe { PERCPU.user_rsp = rsp; } }
pub fn save_user_rflags(rf: u64) { unsafe { PERCPU.user_rflags = rf; } }
pub fn update_kernel_stack(top: u64) { unsafe { PERCPU.kernel_rsp = top; } }

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
