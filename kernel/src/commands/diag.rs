use crate::framebuffer::Console;
use super::CmdResult;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.print_colored("Usage: diag <subcommand>\n", crate::framebuffer::COLOR_YELLOW);
        console.print("  pmm        - test PMM alloc/free\n");
        console.print("  paging     - test page table walker\n");
        console.print("  paging-test - test page table builder\n");
        console.print("  wxtest     - verify .rodata mapping is read-only\n");
            console.print("  fault-test - trigger a fatal kernel page fault (halts VM)\n");
        console.print("  panic-test - test fatal error screen (halts VM)\n");
        return CmdResult::Ok;
    }

    match args[0] {
        "pmm" => diag_pmm(console),
        "paging" => diag_paging(console),
        "paging-test" => diag_paging_test(console),
        "wxtest" => diag_wxtest(console),
        "fault-test" => diag_fault_test(console),
        "panic-test" => panic!("diagnostic panic screen test"),
        _ => {
            console.print_colored("Unknown diag: ", crate::framebuffer::COLOR_RED);
            console.print(args[0]);
            console.print("\n");
        }
    }
    CmdResult::Ok
}

fn diag_pmm(console: &mut Console) {
    console.print_colored("[PMM TEST] Running...\n", crate::framebuffer::COLOR_YELLOW);

    let (used_before, total_before) = crate::memory::pmm::stats();
    const FRAME_BATCH: usize = 128;
    let mut frames = [0usize; FRAME_BATCH];
    let mut allocated = 0usize;
    let mut frames_ok = true;

    console.print("  Before: used=");
    console.print_num(used_before);
    console.print(" total=");
    console.print_num(total_before);
    console.print("\n");

    for _ in 0..FRAME_BATCH {
        if let Some(frame) = crate::memory::pmm::alloc_frame() {
            if frame % crate::memory::pmm::FRAME_SIZE != 0
                || frames[..allocated].contains(&frame)
            {
                frames_ok = false;
            }
            frames[allocated] = frame;
            allocated += 1;
        } else {
            frames_ok = false;
            break;
        }
    }

    let (used_mid, _) = crate::memory::pmm::stats();
    console.print("  After alloc 128: used=");
    console.print_num(used_mid);
    console.print("\n");

    for &frame in &frames[..allocated] {
        crate::memory::pmm::free_frame(frame);
    }

    let contiguous_count = 16;
    let mut contiguous_ok = crate::memory::pmm::alloc_contiguous(0).is_none();
    if let Some(base) = crate::memory::pmm::alloc_contiguous(contiguous_count) {
        let (used_contiguous, _) = crate::memory::pmm::stats();
        contiguous_ok &= base % crate::memory::pmm::FRAME_SIZE == 0;
        contiguous_ok &= used_contiguous == used_before + contiguous_count;
        contiguous_ok &= crate::memory::pmm::free_contiguous(base, contiguous_count);
    } else {
        contiguous_ok = false;
    }

    let (dma_used_before, _) = crate::memory::pmm::stats();
    let dma_ok = if let Some(mut buffer) = crate::memory::dma::DmaBuffer::allocate(8193) {
        let expected_virtual = crate::memory::paging::HHDM_OFFSET.load(
            core::sync::atomic::Ordering::Relaxed,
        ) as usize + buffer.physical_address();
        let valid_layout = buffer.physical_address() % crate::memory::pmm::FRAME_SIZE == 0
            && buffer.as_mut_ptr() as usize == expected_virtual
            && buffer.len() == 8193
            && buffer.allocated_len() == 3 * crate::memory::pmm::FRAME_SIZE
            && buffer.as_slice().iter().all(|byte| *byte == 0);
        if valid_layout {
            buffer.as_mut_slice()[0] = 0xA5;
            buffer.as_mut_slice()[8192] = 0x5A;
        }
        drop(buffer);
        let (dma_used_after, _) = crate::memory::pmm::stats();
        valid_layout && dma_used_before == dma_used_after
    } else {
        false
    };

    let (used_after, total_after) = crate::memory::pmm::stats();
    console.print("  After free: used=");
    console.print_num(used_after);
    console.print(" total=");
    console.print_num(total_after);
    console.print("\n");
    console.print("  DMA buffer: ");
    console.print(if dma_ok { "PASS\n" } else { "FAIL\n" });

    if frames_ok
        && allocated == FRAME_BATCH
        && used_mid == used_before + FRAME_BATCH
        && contiguous_ok
        && dma_ok
        && used_before == used_after
        && total_before == total_after
    {
        console.print_colored("[PMM TEST] PASS\n", crate::framebuffer::COLOR_GREEN);
    } else {
        console.print_colored("[PMM TEST] FAIL (allocation invariant or leak)\n", crate::framebuffer::COLOR_RED);
    }
}

fn diag_paging(console: &mut Console) {
    use crate::memory::paging;

    console.print_colored("[PAGING TEST] Current CR3...\n", crate::framebuffer::COLOR_YELLOW);

    let root = paging::current_root_phys();
    console.print("  CR3 root: 0x");
    console.print_num(root as usize);
    console.print("\n");

    let start_virt = _start as u64;
    match paging::translate(start_virt) {
        Some(mapping) => {
            console.print("  _start: virt=0x");
            console.print_num(start_virt as usize);
            console.print(" -> phys=0x");
            console.print_num(mapping.physical_address as usize);
            console.print("\n");
            console.print_colored("[PAGING TEST] PASS\n", crate::framebuffer::COLOR_GREEN);
        }
        None => {
            console.print_colored("[PAGING TEST] FAIL (translate returned None)\n", crate::framebuffer::COLOR_RED);
        }
    }
}

fn diag_paging_test(console: &mut Console) {
    use crate::memory::paging;

    console.print_colored("[PAGING TEST] sandbox mapping test...\n", crate::framebuffer::COLOR_YELLOW);

    match paging::sandbox_mapping_test() {
        Ok(result) => {
            console.print("  root_phys=0x");
            console.print_num(result.root_phys as usize);
            console.print("\n");
            console.print("  virtual=0x");
            console.print_num(result.virtual_address as usize);
            console.print("\n");
            console.print("  physical=0x");
            console.print_num(result.physical_address as usize);
            console.print("\n");
            console.print_colored("[PAGING TEST] PASS\n", crate::framebuffer::COLOR_GREEN);
        }
        Err(e) => {
            console.print("  Error: ");
            console.print(e);
            console.print("\n");
            console.print_colored("[PAGING TEST] FAIL\n", crate::framebuffer::COLOR_RED);
        }
    }
}

fn diag_wxtest(console: &mut Console) {
    console.print_colored("[W^X TEST] Checking .rodata page permissions...\n", crate::framebuffer::COLOR_YELLOW);

    if crate::memory::vmm::is_ready() {
        let rodata_addr = core::ptr::addr_of!(__rodata_start) as u64;
        console.print("  .rodata addr: 0x");
        console.print_num(rodata_addr as usize);
        console.print("\n");

        match crate::memory::paging::translate(rodata_addr) {
            Some(mapping) => {
                console.print("  PTE flags: 0x");
                console.print_num(mapping.flags as usize);
                console.print("\n");
                if mapping.flags & crate::memory::paging::ENTRY_WRITABLE == 0 {
                    console.print_colored("[W^X TEST] PASS (read-only mapping)\n", crate::framebuffer::COLOR_GREEN);
                } else {
                    console.print_colored("[W^X TEST] FAIL (mapping is writable)\n", crate::framebuffer::COLOR_RED);
                }
            }
            None => {
                console.print_colored("[W^X TEST] FAIL (.rodata is unmapped)\n", crate::framebuffer::COLOR_RED);
            }
        }
    } else {
        console.print_colored("  VMM not active yet. Skipping test.\n", crate::framebuffer::COLOR_YELLOW);
        console.print_colored("[W^X TEST] SKIP\n", crate::framebuffer::COLOR_YELLOW);
    }
}

fn diag_fault_test(console: &mut Console) {
    console.print_colored("[FAULT TEST] This will halt the guest VM.\n", crate::framebuffer::COLOR_YELLOW);
    if !crate::memory::vmm::is_ready() {
        console.print_colored("[FAULT TEST] SKIP (VMM is not active)\n", crate::framebuffer::COLOR_YELLOW);
        return;
    }

    let rodata_addr = core::ptr::addr_of!(__rodata_start) as *mut u8;
    console.print("  Triggering write protection fault at .rodata\n");
    unsafe { core::ptr::write_volatile(rodata_addr, 0xFF); }
    console.print_colored("[FAULT TEST] FAIL (write was allowed)\n", crate::framebuffer::COLOR_RED);
}

extern "C" {
    static __rodata_start: u8;
    fn _start();
}
