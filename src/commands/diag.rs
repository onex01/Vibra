use crate::framebuffer::Console;
use super::CmdResult;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.print_colored("Usage: diag <subcommand>\n", crate::framebuffer::COLOR_YELLOW);
        console.print("  pmm        - test PMM alloc/free\n");
        console.print("  paging     - test page table walker\n");
        console.print("  paging-test - test page table builder\n");
        console.print("  wxtest     - test W^X protection (write to .rodata)\n");
        return CmdResult::Ok;
    }

    match args[0] {
        "pmm" => diag_pmm(console),
        "paging" => diag_paging(console),
        "paging-test" => diag_paging_test(console),
        "wxtest" => diag_wxtest(console),
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
    console.print("  Before: used=");
    console.print_num(used_before);
    console.print(" total=");
    console.print_num(total_before);
    console.print("\n");

    let mut allocated = alloc::vec::Vec::new();
    for _ in 0..100 {
        if let Some(frame) = crate::memory::pmm::alloc_frame() {
            allocated.push(frame);
        }
    }

    let (used_mid, _) = crate::memory::pmm::stats();
    console.print("  After alloc 100: used=");
    console.print_num(used_mid);
    console.print("\n");

    for frame in allocated {
        crate::memory::pmm::free_frame(frame);
    }

    let (used_after, total_after) = crate::memory::pmm::stats();
    console.print("  After free: used=");
    console.print_num(used_after);
    console.print(" total=");
    console.print_num(total_after);
    console.print("\n");

    if used_before == used_after && total_before == total_after {
        console.print_colored("[PMM TEST] PASS\n", crate::framebuffer::COLOR_GREEN);
    } else {
        console.print_colored("[PMM TEST] FAIL (leak detected)\n", crate::framebuffer::COLOR_RED);
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
    console.print_colored("[W^X TEST] Testing write to .rodata...\n", crate::framebuffer::COLOR_YELLOW);
    console.print("  If VMM is working, this should cause a PAGE FAULT.\n");
    console.print("  The PAGE FAULT handler will print the fault info.\n");
    console.print("  After the fault, shell should still work.\n\n");

    if crate::memory::vmm::is_ready() {
        console.print_colored("  VMM is active. Attempting write to .rodata...\n", crate::framebuffer::COLOR_YELLOW);

        let rodata_addr = unsafe {
            core::ptr::addr_of!(__rodata_start) as u64
        };
        console.print("  .rodata addr: 0x");
        console.print_num(rodata_addr as usize);
        console.print("\n");

        // This WILL cause PAGE FAULT - that's the point of the test
        unsafe {
            let ptr = rodata_addr as *mut u8;
            core::ptr::write_volatile(ptr, 0xFF);
        }

        console.print_colored("  ERROR: Write to .rodata did not cause PAGE FAULT!\n", crate::framebuffer::COLOR_RED);
        console.print_colored("[W^X TEST] FAIL\n", crate::framebuffer::COLOR_RED);
    } else {
        console.print_colored("  VMM not active yet. Skipping test.\n", crate::framebuffer::COLOR_YELLOW);
        console.print_colored("[W^X TEST] SKIP\n", crate::framebuffer::COLOR_YELLOW);
    }
}

extern "C" {
    static __rodata_start: u8;
    fn _start();
}
