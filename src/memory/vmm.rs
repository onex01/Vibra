// Virtual Memory Manager (VMM).
//
// Стратегия: копируем PML4 Limine и переключаемся.
// Limine правильно маппит всё (HHDM, lower-half, kernel, framebuffer).
// W^X добавим позже после детального анализа Limine page table layout.

use crate::println;
use crate::memory::pmm;
use crate::memory::paging;
use core::sync::atomic::{AtomicBool, Ordering};
use limine::memmap::Entry;

type PhysAddr = u64;

static NEW_PML4: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static VMM_READY: AtomicBool = AtomicBool::new(false);

// ===== MSR helpers =====

unsafe fn enable_efer_nxe() {
    const EFER_MSR: u32 = 0xC0000080;
    const NXE_BIT: u64 = 1 << 11;
    let mut lo: u32; let mut hi: u32;
    core::arch::asm!("rdmsr", in("ecx") EFER_MSR, out("eax") lo, out("edx") hi);
    let mut efer = ((hi as u64) << 32) | (lo as u64);
    if efer & NXE_BIT == 0 {
        efer |= NXE_BIT;
        core::arch::asm!("wrmsr", in("ecx") EFER_MSR, in("eax") efer as u32, in("edx") (efer >> 32) as u32);
        println!("[VMM] EFER.NXE enabled");
    } else {
        println!("[VMM] EFER.NXE already set");
    }
}

unsafe fn enable_cr0_wp() {
    const WP_BIT: u64 = 1 << 16;
    let cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
    if cr0 & WP_BIT == 0 {
        core::arch::asm!("mov cr0, {0}", in(reg) cr0 | WP_BIT, options(nomem, nostack));
        println!("[VMM] CR0.WP enabled");
    } else {
        println!("[VMM] CR0.WP already set");
    }
}

// ===== Builder: copy Limine PML4 =====

pub fn build_kernel_space(
    _memory_map: &[&Entry],
    hhdm: u64,
    _kernel_phys: u64,
    _kernel_virt: u64,
    _framebuffer_phys: u64,
    _framebuffer_size: u64,
) -> Option<PhysAddr> {
    println!("[VMM] Building kernel address space...");

    let current_cr3_phys = paging::current_root_phys();
    let current_pml4_virt = hhdm + current_cr3_phys;

    let new_pml4_virt = pmm::alloc_frame_zeroed(hhdm)? as *mut u64;
    let new_pml4_phys = new_pml4_virt as u64 - hhdm;
    let src = current_pml4_virt as *const u64;

    unsafe {
        for i in 0..512usize {
            core::ptr::write_volatile(new_pml4_virt.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }
    println!("[VMM] Copied Limine PML4 ({:#x})", new_pml4_phys);
    Some(new_pml4_phys)
}

pub unsafe fn activate(new_pml4_phys: PhysAddr) {
    println!("[VMM] Activating new address space...");
    enable_efer_nxe();
    enable_cr0_wp();

    core::arch::asm!("mov cr3, {0}", in(reg) new_pml4_phys, options(nomem, nostack));
    println!("[VMM] CR3 switched to {:#x}", new_pml4_phys);

    VMM_READY.store(true, Ordering::SeqCst);
}

pub fn new_pml4_phys() -> u64 { NEW_PML4.load(Ordering::Relaxed) }
pub fn is_ready() -> bool { VMM_READY.load(Ordering::SeqCst) }

pub fn init(
    memory_map: &[&Entry],
    hhdm: u64,
    kernel_phys: u64,
    kernel_virt: u64,
    framebuffer_phys: u64,
    framebuffer_size: u64,
) -> Option<PhysAddr> {
    println!("[VMM] Initializing Virtual Memory Manager...");
    let pml4 = build_kernel_space(memory_map, hhdm, kernel_phys, kernel_virt, framebuffer_phys, framebuffer_size)?;
    unsafe { activate(pml4); }
    Some(pml4)
}
