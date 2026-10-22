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
    let efer = ((hi as u64) << 32) | (lo as u64);

    if efer & NXE_BIT != 0 {
        println!("[VMM] EFER.NXE already set");
        return;
    }

    // WRMSR в EFER с NXE=1 на процессоре без NX (CPUID.80000001H:EDX[20]=0,
    // например когда в BIOS выключен «Execute Disable Bit») даёт #GP(0).
    // Без этой проверки ядро падало здесь же на реальном ноутбуке, хотя в
    // QEMU с NX всё работало. NX — оптимизация W^X, а не требование boot.
    if !paging::cpu_has_nx() {
        println!(
            "[VMM] EFER={:#x}: CPU has no NX (CPUID.80000001H:EDX[20]=0) — NX disabled, continuing",
            efer
        );
        return;
    }

    let new_efer = efer | NXE_BIT;
    core::arch::asm!("wrmsr", in("ecx") EFER_MSR, in("eax") new_efer as u32, in("edx") (new_efer >> 32) as u32);
    println!("[VMM] EFER.NXE enabled");
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

    // Страховки: PML4 обязан быть выровнен на 4 КиБ и не лежать в первом
    // мегабайте — иначе CR3 указывает в IVT/BDA, и машина умирает без отчёта.
    if new_pml4_phys == 0 || new_pml4_phys & 0xFFF != 0 || new_pml4_phys < 0x10_0000 {
        println!("[VMM] ERROR: bogus PML4 frame {:#x}", new_pml4_phys);
        return None;
    }

    let src = current_pml4_virt as *const u64;
    let mut copied = 0usize;

    unsafe {
        for i in 0..512usize {
            let entry = core::ptr::read_volatile(src.add(i));
            if entry & paging::ENTRY_PRESENT != 0 {
                copied += 1;
            }
            core::ptr::write_volatile(new_pml4_virt.add(i), entry);
        }
    }

    println!(
        "[VMM] Copied Limine {} ({:#x}), {} present entries",
        if paging::la57_enabled() { "PML5" } else { "PML4" },
        new_pml4_phys,
        copied
    );

    if copied == 0 {
        println!("[VMM] ERROR: source root has no present entries — refusing");
        return None;
    }

    NEW_PML4.store(new_pml4_phys, Ordering::SeqCst);
    Some(new_pml4_phys)
}

/// Переключение CR3 — только после того, как новое адресное пространство
/// подтвердило все критичные маппинги. Иначе `mov cr3` теряет стек/serial/GDT
/// и машина уходит в triple fault без единого сообщения (реальный hang на
/// ноутбуках). При провале CR3 не меняется и возвращается `false`.
pub unsafe fn activate(new_pml4_phys: PhysAddr, regions: &[(&'static str, u64)]) -> bool {
    println!("[VMM] Activating new address space...");
    enable_efer_nxe();
    enable_cr0_wp();

    let (mapped, failed) = paging::validate_space(new_pml4_phys, regions);
    if failed != 0 || mapped == 0 {
        println!(
            "[VMM] FATAL: {} of {} regions unmapped in new space — refusing to switch CR3",
            failed,
            mapped + failed
        );
        println!("[VMM] Keeping Limine page tables so the boot log stays readable");
        return false;
    }

    let old_cr3 = paging::current_root_phys();
    println!(
        "[VMM] All {} regions mapped; switching CR3 {:#x} -> {:#x}",
        mapped, old_cr3, new_pml4_phys
    );

    core::arch::asm!("mov cr3, {0}", in(reg) new_pml4_phys, options(nomem, nostack));

    VMM_READY.store(true, Ordering::SeqCst);
    // Печатаем сразу после switch: этот println! — первая проверка того, что
    // через новый CR3 доступны и стек, и framebuffer, и serial.
    println!("[VMM] CR3 switched to {:#x}", new_pml4_phys);
    true
}

pub fn new_pml4_phys() -> u64 { NEW_PML4.load(Ordering::Relaxed) }
pub fn is_ready() -> bool { VMM_READY.load(Ordering::SeqCst) }

/// Виртуальный адрес активной PML4. При включённом LA57 корень — PML5,
/// и настоящая PML4 для user-адресов берётся из PML5[0].
fn pml4_table_virt() -> *mut u64 {
    let hhdm = paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let cr3_phys = paging::current_root_phys();
    let mut pml4_phys = cr3_phys;
    if paging::la57_enabled() {
        if let Some(entry) = unsafe { paging::read_entry(cr3_phys, 0) } {
            pml4_phys = entry & paging::ADDRESS_MASK;
        }
    }
    (hhdm + pml4_phys) as *mut u64
}

/// Маппинг 4KB страницы с USER битом на ВСЕХ уровнях (PML4→PDPT→PD→PT).
/// writable: true = RW, false = RO
/// executable: true = NX cleared, false = NX set
pub fn map_user_pages(virt: u64, phys: u64, writable: bool, executable: bool) -> bool {
    let hhdm = paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let pml4 = pml4_table_virt();

    let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
    let pd_idx = ((virt >> 21) & 0x1FF) as usize;
    let pt_idx = ((virt >> 12) & 0x1FF) as usize;

    unsafe {
        // PML4 → PDPT
        let pml4e = core::ptr::read_volatile(pml4.add(pml4_idx));
        let pdpt_phys = if pml4e & paging::ENTRY_PRESENT != 0 {
            // Добавляем USER bit к существующей записи (если ещё нет)
            if pml4e & paging::ENTRY_USER == 0 {
                core::ptr::write_volatile(pml4.add(pml4_idx), pml4e | paging::ENTRY_USER);
            }
            pml4e & 0x000F_FFFF_FFFF_F000
        } else {
            // Выделяем PDPT
            let frame = match pmm::alloc_frame_zeroed(hhdm) {
                Some(f) => f,
                None => return false,
            };
            let frame_phys = frame - hhdm;
            let entry = frame_phys | paging::ENTRY_PRESENT | paging::ENTRY_USER;
            core::ptr::write_volatile(pml4.add(pml4_idx), entry);
            frame_phys
        };

        // PDPT → PD
        let pdpt = (hhdm + pdpt_phys) as *mut u64;
        let pdpt_entry = core::ptr::read_volatile(pdpt.add(pdpt_idx));
        let pd_phys = if pdpt_entry & paging::ENTRY_PRESENT != 0 {
            if pdpt_entry & paging::ENTRY_HUGE_PAGE != 0 {
                // 1GB page — разбиваем на 512 × 2MB
                // Для 1GB pages: phys в битах [51:30], биты [29:12] = available
                let huge_phys = pdpt_entry & 0x0000_007F_C000_0000;
                let pd_frame = match pmm::alloc_frame_zeroed(hhdm) {
                    Some(f) => f,
                    None => return false,
                };
                let pd_phys_new = pd_frame - hhdm;
                let pd_new = (hhdm + pd_phys_new) as *mut u64;

                for i in 0..512usize {
                    let page_phys = huge_phys + (i as u64) * 2 * 1024 * 1024;
                    let flags = paging::ENTRY_PRESENT | paging::ENTRY_USER | paging::ENTRY_WRITABLE | paging::ENTRY_HUGE_PAGE;
                    core::ptr::write_volatile(pd_new.add(i), page_phys | flags);
                }

                let new_pdpt_entry = pd_phys_new | paging::ENTRY_PRESENT | paging::ENTRY_USER | paging::ENTRY_WRITABLE;
                core::ptr::write_volatile(pdpt.add(pdpt_idx), new_pdpt_entry);
                pd_phys_new
            } else {
                let existing = pdpt_entry;
                if existing & paging::ENTRY_USER == 0 {
                    core::ptr::write_volatile(pdpt.add(pdpt_idx), existing | paging::ENTRY_USER);
                }
                existing & 0x000F_FFFF_FFFF_F000
            }
        } else {
            let frame = match pmm::alloc_frame_zeroed(hhdm) {
                Some(f) => f,
                None => return false,
            };
            let frame_phys = frame - hhdm;
            let entry = frame_phys | paging::ENTRY_PRESENT | paging::ENTRY_USER;
            core::ptr::write_volatile(pdpt.add(pdpt_idx), entry);
            frame_phys
        };

        // PD → PT
        let pd = (hhdm + pd_phys) as *mut u64;
        let pd_entry = core::ptr::read_volatile(pd.add(pd_idx));

        let pt_phys = if pd_entry & paging::ENTRY_PRESENT != 0 {
            if pd_entry & paging::ENTRY_HUGE_PAGE != 0 {
                // 2MB huge page — разбиваем на 512 × 4KB
                // Для 2MB pages: phys в битах [51:21], биты [20:12] = available
                let huge_phys = pd_entry & 0x000F_FFFF_FFE0_0000;
                let pt_frame = match pmm::alloc_frame_zeroed(hhdm) {
                    Some(f) => f,
                    None => return false,
                };
                let pt_phys_new = pt_frame - hhdm;
                let pt = (hhdm + pt_phys_new) as *mut u64;

                // Заполняем 512 PTE: каждый指向 4KB внутри 2MB страницы
                for i in 0..512usize {
                    let page_phys = huge_phys + (i as u64) * 4096;
                    let mut flags = paging::ENTRY_PRESENT | paging::ENTRY_USER | paging::ENTRY_WRITABLE;
                    core::ptr::write_volatile(pt.add(i), page_phys | flags);
                }

                // Заменяем PD entry: huge → normal PT pointer
                let new_pd_entry = pt_phys_new | paging::ENTRY_PRESENT | paging::ENTRY_USER | paging::ENTRY_WRITABLE;
                core::ptr::write_volatile(pd.add(pd_idx), new_pd_entry);

                pt_phys_new
            } else {
                pd_entry & 0x000F_FFFF_FFFF_F000
            }
        } else {
            let frame = match pmm::alloc_frame_zeroed(hhdm) {
                Some(f) => f,
                None => return false,
            };
            let frame_phys = frame - hhdm;
            let entry = frame_phys | paging::ENTRY_PRESENT | paging::ENTRY_USER;
            core::ptr::write_volatile(pd.add(pd_idx), entry);
            frame_phys
        };

        // PT → PTE
        let pt = (hhdm + pt_phys) as *mut u64;
        let mut flags = paging::ENTRY_PRESENT | paging::ENTRY_USER;
        if writable { flags |= paging::ENTRY_WRITABLE; }
        flags |= paging::nx_flags(executable);

        let entry = phys | flags;
        core::ptr::write_volatile(pt.add(pt_idx), entry);

        // Полная инвалидация TLB: перезагружаем CR3
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack));
    }

    true
}

/// Маппинг страницы через СВОБОДНЫЙ PML4 slot (избегает Limine 2MB pages).
/// Находит первый свободный PML4 entry и создаёт всю иерархию с нуля.
/// Возвращает виртуальный адрес или None.
pub fn map_user_page_fresh(phys: u64, writable: bool, executable: bool) -> Option<u64> {
    let hhdm = paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let pml4 = pml4_table_virt();

    unsafe {
        // Ищем свободный PML4 slot (2..255, пропускаем 0 и 1 — lower half)
        let mut free_slot: Option<usize> = None;
        for slot in 2..256usize {
            let entry = core::ptr::read_volatile(pml4.add(slot));
            if entry & paging::ENTRY_PRESENT == 0 {
                free_slot = Some(slot);
                break;
            }
        }

        let slot = free_slot?;
        let virt = (slot as u64) << 39; // canonical address для этого slot

        // Выделяем PDPT
        let pdpt_frame = pmm::alloc_frame_zeroed(hhdm)?;
        let pdpt_phys = pdpt_frame - hhdm;
        core::ptr::write_volatile(pml4.add(slot),
            pdpt_phys | paging::ENTRY_PRESENT | paging::ENTRY_WRITABLE | paging::ENTRY_USER);

        // Выделяем PD
        let pd_frame = pmm::alloc_frame_zeroed(hhdm)?;
        let pd_phys = pd_frame - hhdm;
        let pdpt = (hhdm + pdpt_phys) as *mut u64;
        core::ptr::write_volatile(pdpt.add(0),
            pd_phys | paging::ENTRY_PRESENT | paging::ENTRY_WRITABLE | paging::ENTRY_USER);

        // Выделяем PT
        let pt_frame = pmm::alloc_frame_zeroed(hhdm)?;
        let pt_phys = pt_frame - hhdm;
        let pd = (hhdm + pd_phys) as *mut u64;
        core::ptr::write_volatile(pd.add(0),
            pt_phys | paging::ENTRY_PRESENT | paging::ENTRY_WRITABLE | paging::ENTRY_USER);

        // PTE: phys → virt (identity в рамках этого slot)
        let pt = (hhdm + pt_phys) as *mut u64;
        let mut flags = paging::ENTRY_PRESENT | paging::ENTRY_USER;
        if writable { flags |= paging::ENTRY_WRITABLE; }
        flags |= paging::nx_flags(executable);
        core::ptr::write_volatile(pt.add(0), phys | flags);

        // TLB flush
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack));

        Some(virt)
    }
}

pub fn init(
    memory_map: &[&Entry],
    hhdm: u64,
    kernel_phys: u64,
    kernel_virt: u64,
    framebuffer_phys: u64,
    framebuffer_size: u64,
    regions: &[(&'static str, u64)],
) -> Option<PhysAddr> {
    println!("[VMM] Initializing Virtual Memory Manager...");
    let pml4 = build_kernel_space(memory_map, hhdm, kernel_phys, kernel_virt, framebuffer_phys, framebuffer_size)?;
    if !unsafe { activate(pml4, regions) } {
        return None;
    }
    Some(pml4)
}
