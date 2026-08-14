use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::Ordering;

// Маленькая инфраструктура xHCI: выделение Admin Submission/Completion queues
// и запись AQA/ASQ/ACQ регистров. Это подготовительный этап для отправки
// команд (Enable Slot и т.д.).

pub struct XhciController {
    pub vendor: u16,
    pub device: u16,
    pub bar_phys: u64,
    pub op_base_virt: u64,
    pub asq_phys: usize,
    pub acq_phys: usize,
    pub asq_virt: usize,
    pub acq_virt: usize,
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

        // Register this controller for later polling/debugging
        register_controller(bar_phys, op_base, asq_phys, acq_phys, asq_virt, acq_virt);

        // Note: ASQ/ACQ doorbells are not set yet; further steps required.
    }
}

static XHCI_CONTROLLERS: Mutex<Vec<XhciController>> = Mutex::new(Vec::new());

/// Add controller metadata to global list for later polling/processing.
fn register_controller(bar_phys: u64, op_base_virt: u64, asq_phys: usize, acq_phys: usize, asq_virt: usize, acq_virt: usize) {
    let c = XhciController {
        vendor: 0,
        device: 0,
        bar_phys,
        op_base_virt,
        asq_phys,
        acq_phys,
        asq_virt,
        acq_virt,
    };
    XHCI_CONTROLLERS.lock().push(c);
}

/// Простейшая проверка: считать первые 16 байт ACQ для всех контроллеров
/// и залогировать их как u64/u64. Это даст понимание, пишет ли контроллер
/// Command Completion TRB в выделенный буфер.
pub fn poll_admin_acq() {
    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        println!("[xHCI] poll_admin_acq: controller bar={:#x} acq_phys={:#x} acq_virt={:#x}", c.bar_phys, c.acq_phys, c.acq_virt);
        unsafe {
            let p0 = c.acq_virt as *const u64;
            let p1 = (c.acq_virt + 8) as *const u64;
            let v0 = core::ptr::read_volatile(p0);
            let v1 = core::ptr::read_volatile(p1);
            println!("[xHCI] ACQ TRB0: {:#x} {:#x}", v0, v1);
        }
    }
}

/// Записать тестовую TRB в ASQ для первого контроллера (локально, без doorbell)
pub fn submit_dummy_command() {
    let mut list = XHCI_CONTROLLERS.lock();
    if list.is_empty() {
        println!("[xHCI] submit_dummy_command: no controllers registered");
        return;
    }
    // Возьмём первый контроллер
    let c = &mut list[0];
    println!("[xHCI] submit_dummy_command: writing TRB to ASQ virt={:#x}", c.asq_virt);
    unsafe {
        let p0 = c.asq_virt as *mut u64;
        let p1 = (c.asq_virt + 8) as *mut u64;
        core::ptr::write_volatile(p0, 0xdead_beef_dead_beefu64);
        core::ptr::write_volatile(p1, 0x1122_3344_5566_7788u64);
    }
    println!("[xHCI] dummy TRB written to ASQ");
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
