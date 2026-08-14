use crate::println;
use core::sync::atomic::Ordering;

// Маленькая инфраструктура xHCI: выделение Admin Submission/Completion queues
// и запись AQA/ASQ/ACQ регистров. Это подготовительный этап для отправки
// команд (Enable Slot и т.д.).

pub struct XhciController {
    pub vendor: u16,
    pub device: u16,
    pub bar_phys: u64,
    pub op_base_virt: u64,
}

fn hhdm() -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed)
}

fn alloc_contiguous_page() -> Option<usize> {
    // Allocate a single 4K frame contiguous (phys address)
    crate::memory::pmm::alloc_contiguous(1)
}

pub fn init_controller(bar_phys: u64, mmio_virt: u64) {
    println!("[xHCI] init_controller: bar_phys={:#x} mmio_virt={:#x}", bar_phys, mmio_virt);

    // CAPLENGTH at mmio_virt[0]
    unsafe {
        let cap_len = core::ptr::read_volatile(mmio_virt as *const u8) as u64;
        let op_base = mmio_virt + cap_len;
        println!("[xHCI] cap_len={} op_base={:#x}", cap_len, op_base);

        // Allocate ASQ and ACQ (one page each)
        let asq_phys = match alloc_contiguous_page() {
            Some(p) => p,
            None => { println!("[xHCI] failed to allocate ASQ phys"); return; }
        };
        let acq_phys = match alloc_contiguous_page() {
            Some(p) => p,
            None => { println!("[xHCI] failed to allocate ACQ phys"); return; }
        };

        let asq_virt = hhdm() as usize + asq_phys;
        let acq_virt = hhdm() as usize + acq_phys;

        println!("[xHCI] ASQ phys={:#x} virt={:#x}", asq_phys, asq_virt);
        println!("[xHCI] ACQ phys={:#x} virt={:#x}", acq_phys, acq_virt);

        // AQA at offset 0x24 (u32): bits [15:0] = ASQ size -1, bits [31:16] = ACQ size-1
        let aqa_ptr = (op_base + 0x24) as *mut u32;
        // For now choose size 0x0 (meaning 1 entry) to be conservative
        let asq_entries = 0x1u32; // ASQ size - 1 => 1 => 2 entries? keep small
        let acq_entries = 0x1u32;
        let aqa_val: u32 = ((acq_entries & 0xffff) << 16) | (asq_entries & 0xffff);
        core::ptr::write_volatile(aqa_ptr, aqa_val);
        println!("[xHCI] wrote AQA={:#x} to {:#x}", aqa_val, aqa_ptr as u64);

        // ASQ pointer at 0x28 (64-bit)
        let asq_ptr_lo = (op_base + 0x28) as *mut u32;
        let asq_ptr_hi = (op_base + 0x2C) as *mut u32;
        core::ptr::write_volatile(asq_ptr_lo, asq_phys as u32);
        core::ptr::write_volatile(asq_ptr_hi, (asq_phys >> 32) as u32);
        println!("[xHCI] wrote ASQ ptr {:#x}", asq_phys);

        // ACQ pointer at 0x30 (64-bit)
        let acq_ptr_lo = (op_base + 0x30) as *mut u32;
        let acq_ptr_hi = (op_base + 0x34) as *mut u32;
        core::ptr::write_volatile(acq_ptr_lo, acq_phys as u32);
        core::ptr::write_volatile(acq_ptr_hi, (acq_phys >> 32) as u32);
        println!("[xHCI] wrote ACQ ptr {:#x}", acq_phys);

        // Note: ASQ/ACQ doorbells are not set yet; further steps required.
    }
}

/// Выделить Event Ring и структуру ERST (Event Ring Segment Table).
/// Пока что только выделение памяти и логирование адресов — запись
/// регистров RT (runtime) будет в следующих итерациях.
pub fn init_event_ring(bar_phys: u64, mmio_virt: u64) {
    println!("[xHCI] init_event_ring: allocating event ring memory");
    // Allocate one page for event ring buffer and one for ERST
    if let Some(ev_phys) = crate::memory::pmm::alloc_contiguous(1) {
        let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        let ev_virt = h as usize + ev_phys;
        println!("[xHCI] Event ring phys={:#x} virt={:#x}", ev_phys, ev_virt);
    } else {
        println!("[xHCI] Failed to allocate event ring page");
    }

    if let Some(erst_phys) = crate::memory::pmm::alloc_contiguous(1) {
        let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        let erst_virt = h as usize + erst_phys;
        println!("[xHCI] ERST table phys={:#x} virt={:#x}", erst_phys, erst_virt);
    } else {
        println!("[xHCI] Failed to allocate ERST page");
    }
}
