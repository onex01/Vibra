use super::CmdResult;
use alloc::format;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use crate::memory::pmm;
use crate::memory::paging;

// Диагностические тесты ядра. Использование: diag <test>
pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    match args.first().copied() {
        Some("dftest") => {
            console.print_colored("Triggering double fault (check serial output)...\n", COLOR_RED);
            unsafe {
                // Портим RSP и делаем push: #PF на битом стеке -> CPU не может
                // положить фрейм -> #DF. Если IST работает, обработчик
                // напечатает DOUBLE FAULT в serial вместо triple-fault ребута.
                core::arch::asm!("mov rsp, 0x10", "push rax", options(noreturn));
            }
        }
        Some("pmtest") => pmtest(console),
        Some("paging") => paging_report(console),
        Some("paging-test") => paging_test(console),
        _ => {
            console.print_colored("Kernel diagnostics. Usage: diag <test>\n", COLOR_CYAN);
            console.print_colored("  dftest   - force a double fault (tests IST stack; HALTS system!)\n", COLOR_YELLOW);
            console.print_colored("  pmtest   - PMM alloc/free/contiguous/stats test\n", COLOR_YELLOW);
            console.print_colored("  paging   - show active CR3 and verify address mappings\n", COLOR_YELLOW);
            console.print_colored("  paging-test - build and verify an inactive 4 KiB mapping\n", COLOR_YELLOW);
            CmdResult::Ok
        }
    }
}

fn paging_test(console: &mut Console) -> CmdResult {
    console.print_colored("Paging sandbox test (CR3 is not switched)\n", COLOR_CYAN);
    match paging::sandbox_mapping_test() {
        Ok(mapping) => {
            crate::println!(
                "[PAGING] sandbox PASS: root={:#x}, virt={:#x} -> phys={:#x}",
                mapping.root_phys,
                mapping.virtual_address,
                mapping.physical_address
            );
            console.print_colored("  PASS: copied PML4 maps a private 4 KiB page\n", COLOR_GREEN);
            console.print(&format!(
                "  temporary root={:#x}, virt={:#x} -> phys={:#x}\n",
                mapping.root_phys,
                mapping.virtual_address,
                mapping.physical_address
            ));
        }
        Err(error) => {
            crate::println!("[PAGING] sandbox FAIL: {}", error);
            console.print_colored("  FAIL: ", COLOR_RED);
            console.print(error);
            console.print("\n");
        }
    }
    CmdResult::Ok
}

fn paging_report(console: &mut Console) -> CmdResult {
    let root = paging::current_root_phys();
    console.print_colored("Paging diagnostics (read-only)\n", COLOR_CYAN);
    console.print(&format!("  CR3 root: {:#x}\n", root));

    let kernel_address = crate::_start as *const () as usize as u64;
    match paging::translate(kernel_address) {
        Some(mapping) => {
            crate::println!(
                "[PAGING] diag: _start virt={:#x} -> phys={:#x} ({:?})",
                kernel_address,
                mapping.physical_address,
                mapping.page_size
            );
            console.print_colored("  kernel _start: ", COLOR_YELLOW);
            console.print(&format!(
                "virt={:#x} -> phys={:#x}, page={} KiB, flags={:#x}\n",
                kernel_address,
                mapping.physical_address,
                mapping.page_size.bytes() / 1024,
                mapping.flags
            ));
            console.print_colored("  PASS: active kernel mapping is present\n", COLOR_GREEN);
        }
        None => {
            crate::println!("[PAGING] diag: FAIL, _start is not mapped");
            console.print_colored("  FAIL: kernel _start is not mapped\n", COLOR_RED);
        }
    }
    CmdResult::Ok
}

// Тест PMM Шага 2:
//   1) stats до -> alloc_contiguous(16) -> 1000x alloc_frame -> free всё -> stats после.
//   Счётчики used должны совпасть в начале и в конце (доказывает корректность
//  free + next-fit + contiguous). Возвращает CmdResult::Ok всегда.
fn pmtest(console: &mut Console) -> CmdResult {
    const N: usize = 1000;

    let (used_before, total) = pmm::stats();
    console.print_colored(
        "PMM test: starting (allocate contiguous + 1000 frames, then free all)\n",
        COLOR_CYAN,
    );
    console.print(&format!("  before: used={}, total={}\n", used_before, total));

    // Contiguous: 16 подряд идущих фреймов.
    let contig = pmm::alloc_contiguous(16);
    match contig {
        Some(addr) => console.print_colored(
            &format!("  alloc_contiguous(16) -> {:#x} OK\n", addr),
            COLOR_GREEN,
        ),
        None => {
            console.print_colored("  alloc_contiguous(16) FAILED\n", COLOR_RED);
            return CmdResult::Ok;
        }
    }

    // 1000 одиночных фреймов в стек-массив (без heap — он ещё bump).
    let mut frames = [0usize; N];
    let mut allocated = 0usize;
    for slot in frames.iter_mut() {
        match pmm::alloc_frame() {
            Some(a) => { *slot = a; allocated += 1; }
            None => break,
        }
    }
    console.print(&format!("  alloc_frame() x{} / {}\n", allocated, N));

    // Освобождаем всё в обратном порядке.
    for slot in frames.iter().take(allocated) {
        pmm::free_frame(*slot);
    }
    if let Some(addr) = contig {
        if !pmm::free_contiguous(addr, 16) {
            console.print_colored("  FAIL: could not free contiguous range\n", COLOR_RED);
            return CmdResult::Ok;
        }
    }

    let (used_after, total_after) = pmm::stats();
    console.print(&format!("  after:  used={}, total={}\n", used_after, total_after));

    if used_after == used_before {
        console.print_colored("  PASS: counters restored\n", COLOR_GREEN);
    } else {
        console.print_colored(
            &format!("  FAIL: used drifted {} -> {}\n", used_before, used_after),
            COLOR_RED,
        );
    }
    CmdResult::Ok
}
