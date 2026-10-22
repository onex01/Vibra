use crate::println;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::{Ordering, AtomicBool};

// Маленькая инфраструктура xHCI: выделение Admin Submission/Completion queues
// и запись AQA/ASQ/ACQ регистров. Это подготовительный этап для отправки
// команд (Enable Slot и т.д.).

pub struct XhciController {
    pub vendor: u16,
    pub device: u16,
    pub bar_phys: u64,
    pub mmio_base_virt: u64,
    pub op_base_virt: u64,
    pub asq_phys: usize,
    pub acq_phys: usize,
    pub asq_virt: usize,
    pub acq_virt: usize,
    pub event_ring_phys: Option<usize>,
    pub event_ring_virt: Option<usize>,
    pub erst_phys: Option<usize>,
    pub erst_virt: Option<usize>,
    pub cap_rt_off: Option<u32>,
    pub asq_idx: usize,
    pub asq_cycle: u8,
    pub asq_size: usize,
}

fn hhdm() -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed)
}

fn alloc_contiguous_page() -> Option<usize> {
    // Allocate a single 4K frame contiguous (phys address)
    crate::memory::pmm::alloc_contiguous(1)
}

/// Zeroed физический фрейм (возвращает phys). Нужен для DCBAA/CRCR: контроллер
/// читает их сам, поэтому виртуальный адрес не годится.
fn alloc_zeroed_frame() -> Option<u64> {
    let phys = alloc_contiguous_page()? as u64;
    let virt = hhdm() + phys;
    unsafe {
        let words = virt as *mut u64;
        for i in 0..(crate::memory::pmm::FRAME_SIZE / 8) {
            core::ptr::write_volatile(words.add(i), 0);
        }
    }
    Some(phys)
}

/// Ждёт, пока `mask` в MMIO-регистре станет нулём. Ограничение по итерациям
/// обязательно: без него неотвечающий контроллер вешает boot навсегда.
unsafe fn wait_bits_clear(reg: *const u32, mask: u32, limit: u32) -> bool {
    let mut spins = 0u32;
    while (core::ptr::read_volatile(reg) & mask) != 0 {
        core::hint::spin_loop();
        spins += 1;
        if spins >= limit {
            return false;
        }
    }
    true
}

unsafe fn write_u64(addr: u64, value: u64) {
    core::ptr::write_volatile(addr as *mut u32, value as u32);
    core::ptr::write_volatile((addr + 4) as *mut u32, (value >> 32) as u32);
}

pub fn init_controller(bar_phys: u64, mmio_virt: u64) {
    println!("[xHCI] init_controller: bar_phys={:#x} mmio_virt={:#x}", bar_phys, mmio_virt);

    unsafe {
        // === Capability registers ===
        // 0x00 CAPLENGTH(u8) + HCIVERSION(u16), 0x04 HCSPARAMS1,
        // 0x10 HCCPARAMS1, 0x14 DBOFF, 0x18 RTSOFF.
        let cap_len = core::ptr::read_volatile(mmio_virt as *const u8) as u64;
        let hci_version = core::ptr::read_volatile((mmio_virt + 0x02) as *const u16);
        let hcsparams1 = core::ptr::read_volatile((mmio_virt + 0x04) as *const u32);
        let dboff = core::ptr::read_volatile((mmio_virt + 0x14) as *const u32);
        let rtsoff = core::ptr::read_volatile((mmio_virt + 0x18) as *const u32);

        let max_slots = hcsparams1 & 0xFF;
        let max_ports = (hcsparams1 >> 24) & 0xFF;
        let op_base = mmio_virt + cap_len;

        println!(
            "[xHCI] v{}.{} CAPLEN={} slots={} ports={} DBOFF={:#x} RTSOFF={:#x}",
            hci_version >> 8,
            hci_version & 0xFF,
            cap_len,
            max_slots,
            max_ports,
            dboff,
            rtsoff
        );

        // === Operational registers ===
        // USBCMD 0x00, USBSTS 0x04, CRCR 0x18 (64), DCBAAP 0x30 (64), CONFIG 0x38.
        let usbcmd = op_base as *mut u32;
        let usbsts = (op_base + 0x04) as *const u32;

        // 1. Контроллер должен быть остановлен (USBSTS.HCH, bit 0).
        if !wait_bits_clear(usbsts, 1 << 0, 100_000) {
            println!("[xHCI] controller is not halted, skipping init");
            return;
        }

        // 2. Reset (USBCMD.HCRST, bit 1): бит самоочищается, затем ждём CNR (bit 11).
        let cmd = core::ptr::read_volatile(usbcmd);
        core::ptr::write_volatile(usbcmd, cmd | (1 << 1));
        if !wait_bits_clear(usbcmd, 1 << 1, 1_000_000) {
            println!("[xHCI] reset timeout (HCRST stuck)");
            return;
        }
        if !wait_bits_clear(usbsts, 1 << 11, 1_000_000) {
            println!("[xHCI] reset timeout (Controller Not Ready stuck)");
            return;
        }

        // 3. Device Context Base Address Array (DCBAAP, 0x30).
        let Some(dcbaa_phys) = alloc_zeroed_frame() else {
            println!("[xHCI] no memory for DCBAA");
            return;
        };
        write_u64(op_base + 0x30, dcbaa_phys);

        // 4. Command Ring (CRCR, 0x18) — cycle state = 1.
        let Some(cr_phys) = alloc_zeroed_frame() else {
            println!("[xHCI] no memory for command ring");
            return;
        };
        write_u64(op_base + 0x18, cr_phys | 1);

        // 5. CONFIG.MaxSlotsEn (биты 7:0) — иначе Enable Slot не работает.
        core::ptr::write_volatile((op_base + 0x38) as *mut u32, max_slots & 0xFF);

        // 6. Run (USBCMD.RS, bit 0) и дождаться снятия HCHalted.
        let cmd = core::ptr::read_volatile(usbcmd);
        core::ptr::write_volatile(usbcmd, cmd | 1);
        if !wait_bits_clear(usbsts, 1 << 0, 1_000_000) {
            println!("[xHCI] controller did not leave HCHalted — USB unusable");
            return;
        }

        println!(
            "[xHCI] running: DCBAAP={:#x} CRCR={:#x} CONFIG={:#x}",
            dcbaa_phys,
            cr_phys | 1,
            core::ptr::read_volatile((op_base + 0x38) as *const u32)
        );

        register_controller(
            bar_phys,
            mmio_virt,
            op_base,
            cr_phys as usize,
            0,
            (hhdm() + cr_phys) as usize,
            0,
            Some(rtsoff),
        );
    }
}

static XHCI_CONTROLLERS: Mutex<Vec<XhciController>> = Mutex::new(Vec::new());
static RUNTIME_WRITES_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable or disable actual MMIO writes to runtime/registers (ERST/ERDP/doorbell).
/// Default is disabled; set to true only after verifying runtime base via diagnostics.
pub fn set_runtime_writes_enabled(v: bool) {
    RUNTIME_WRITES_ENABLED.store(v, Ordering::Relaxed);
    println!("[xHCI] set_runtime_writes_enabled = {}", v);
}

/// Add controller metadata to global list for later polling/processing.
fn register_controller(bar_phys: u64, mmio_base_virt: u64, op_base_virt: u64, asq_phys: usize, acq_phys: usize, asq_virt: usize, acq_virt: usize, cap_rt_off: Option<u32>) {
    let c = XhciController {
        vendor: 0,
        device: 0,
        bar_phys,
        mmio_base_virt,
        op_base_virt,
        asq_phys,
        acq_phys,
        asq_virt,
        acq_virt,
        event_ring_phys: None,
        event_ring_virt: None,
        erst_phys: None,
        erst_virt: None,
        cap_rt_off,
        asq_idx: 0,
        asq_cycle: 1,
        asq_size: 8,
    };
    XHCI_CONTROLLERS.lock().push(c);
}

/// Diagnostic: for each registered controller, log a few 32-bit words from the
/// candidate runtime register base computed from the stored RTSOFF value.
pub fn log_runtime_regs() {
    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        if let Some(rt) = c.cap_rt_off {
            // RTSOFF отсчитывается от НАЧАЛА MMIO-бара, а не от operational base.
            let rt_base = c.mmio_base_virt + (rt as u64);
            println!("[xHCI] controller {:#x} runtime candidate={:#x}", c.bar_phys, rt_base);
            unsafe {
                for i in 0..8 {
                    let p = (rt_base + (i * 4) as u64) as *const u32;
                    let v = core::ptr::read_volatile(p);
                    println!("[xHCI] runtime[{:#x}] = {:#x}", i*4, v);
                }
            }
        } else {
            println!("[xHCI] controller {:#x} has no RTSOFF recorded", c.bar_phys);
        }
    }
}

/// Простейшая проверка: считать первые 16 байт ACQ для всех контроллеров
/// и залогировать их как u64/u64. Это даст понимание, пишет ли контроллер
/// Command Completion TRB в выделенный буфер.
pub fn poll_admin_acq() {
    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        if c.acq_virt == 0 {
            // Event ring ещё не настроен — читать нечего.
            continue;
        }
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

/// Сформировать и записать Enable Slot TRB в ASQ (без вызова doorbell).
pub fn submit_enable_slot() {
    let mut list = XHCI_CONTROLLERS.lock();
    if list.is_empty() {
        println!("[xHCI] submit_enable_slot: no controllers registered");
        return;
    }
    let c = &mut list[0];
    println!("[xHCI] submit_enable_slot: enqueueing Enable Slot TRB to ASQ virt={:#x}", c.asq_virt);
    // Build TRB
    let param: u64 = 0;
    // TRB type 0x0C = Enable Slot Command
    let trb_type: u64 = 0x0C;
    let mut ctl: u64 = (trb_type << 10) & 0xffff_ffff_ffffu64;
    // set cycle bit according to current asq_cycle
    if c.asq_cycle & 1 == 1 {
        ctl |= 1u64;
    }

    // Enqueue into ASQ ring (simple, no wrap protection beyond modulo)
    let idx = c.asq_idx % c.asq_size;
    let off = c.asq_virt + idx * 16;
    unsafe {
        let p0 = off as *mut u64;
        let p1 = (off + 8) as *mut u64;
        core::ptr::write_volatile(p0, param);
        core::ptr::write_volatile(p1, ctl);
    }
    println!("[xHCI] ENQ ASQ idx={} off={:#x} param={:#x} ctl={:#x}", idx, off, param, ctl);

    // Advance producer index and flip cycle if wrapped
    c.asq_idx = c.asq_idx.wrapping_add(1);
    if c.asq_idx % c.asq_size == 0 {
        c.asq_cycle ^= 1;
    }

    // Compute doorbell candidate and log. Actual write is gated by RUNTIME_WRITES_ENABLED.
    let doorbell_candidate = c.op_base_virt + 0x1000; // conservative candidate
    let doorbell_val: u32 = 0; // admin ring uses DB = 0
    println!("[xHCI] doorbell candidate={:#x} val={:#x}", doorbell_candidate, doorbell_val);
    if RUNTIME_WRITES_ENABLED.load(Ordering::Relaxed) {
        println!("[xHCI] RUNTIME_WRITES_ENABLED: writing doorbell...");
        unsafe { core::ptr::write_volatile(doorbell_candidate as *mut u32, doorbell_val); }
    } else {
        println!("[xHCI] RUNTIME_WRITES_ENABLED disabled: skipping doorbell write");
    }
}

/// Выделить Event Ring и структуру ERST (Event Ring Segment Table).
/// Пока что только выделение памяти и логирование адресов — запись
/// регистров RT (runtime) будет в следующих итерациях.
pub fn init_event_ring(bar_phys: u64, mmio_virt: u64) {
    println!("[xHCI] init_event_ring: allocating event ring memory");
    // Allocate one page for event ring buffer and one for ERST
    let mut ev_phys_opt: Option<usize> = None;
    let mut erst_phys_opt: Option<usize> = None;
    if let Some(ev_phys) = crate::memory::pmm::alloc_contiguous(1) {
        let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        let ev_virt = h as usize + ev_phys;
        println!("[xHCI] Event ring phys={:#x} virt={:#x}", ev_phys, ev_virt);
        ev_phys_opt = Some(ev_phys);
    } else {
        println!("[xHCI] Failed to allocate event ring page");
    }

    if let Some(erst_phys) = crate::memory::pmm::alloc_contiguous(1) {
        let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        let erst_virt = h as usize + erst_phys;
        println!("[xHCI] ERST table phys={:#x} virt={:#x}", erst_phys, erst_virt);
        erst_phys_opt = Some(erst_phys);
    } else {
        println!("[xHCI] Failed to allocate ERST page");
    }

    // Store allocated event ring / erst into the controller entry if present
    if ev_phys_opt.is_some() || erst_phys_opt.is_some() {
        let mut list = XHCI_CONTROLLERS.lock();
        for c in list.iter_mut() {
            if c.bar_phys == bar_phys {
                if let Some(evp) = ev_phys_opt {
                    let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
                    c.event_ring_phys = Some(evp);
                    c.event_ring_virt = Some(h as usize + evp);
                }
                if let Some(ep) = erst_phys_opt {
                    let h = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
                    c.erst_phys = Some(ep);
                    c.erst_virt = Some(h as usize + ep);
                }
                println!("[xHCI] controller updated with event_ring/erst");
                break;
            }
        }
    }
}

/// Простейший парсер Event Ring: прочитать несколько TRB из выделенного
/// event ring буфера и залогировать их как u64 пары для диагностики.
pub fn poll_event_ring() {
    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        if let Some(ev_virt) = c.event_ring_virt {
            println!("[xHCI] poll_event_ring: controller bar={:#x} event_ring_virt={:#x}", c.bar_phys, ev_virt);
            unsafe {
                // прочитаем первые 4 TRB (каждый TRB 16 байт)
                for i in 0..4 {
                    let off = ev_virt + i * 16;
                    let p0 = off as *const u64;
                    let p1 = (off + 8) as *const u64;
                    let v0 = core::ptr::read_volatile(p0);
                    let v1 = core::ptr::read_volatile(p1);
                    // Попытка распарсить TRB: параметры и контрольные биты
                    let trb_param = v0;
                    let trb_ctl = v1;
                    let trb_type = ((trb_ctl >> 10) & 0x3F) as u8;
                    let completion_code = ((trb_ctl >> 24) & 0xFF) as u8;
                    let cycle = (trb_ctl & 1) as u8;
                    println!("[xHCI] ER TRB{}: param={:#018x} ctl={:#018x} type={} cc={} cycle={}",
                        i, trb_param, trb_ctl, trb_type, completion_code, cycle);

                    // Detect Command Completion Event (type 0x21) and handle it
                    if trb_type == 0x21 {
                        println!("[xHCI] Command Completion Event detected (cc={})", completion_code);
                        // Call handler (non-panicking) to process completion
                        handle_command_completion(c.bar_phys, trb_param, trb_ctl);
                    }
                }
            }
        } else {
            println!("[xHCI] poll_event_ring: controller bar={:#x} no event_ring allocated", c.bar_phys);
        }
    }
}

/// Handle a Command Completion Event TRB. For now only logs and handles Enable Slot success.
fn handle_command_completion(bar_phys: u64, trb_param: u64, trb_ctl: u64) {
    let completion_code = ((trb_ctl >> 24) & 0xFF) as u8;
    // For Command Completion, the TRB parameter usually contains the slot ID for Enable Slot.
    // We'll attempt to extract a plausible Slot ID from bits 24..31 of parameter or from ctl.
    let slot_id = ((trb_param >> 24) & 0xFF) as u8;
    println!("[xHCI] handle_command_completion: controller={:#x} slot_id={} cc={}", bar_phys, slot_id, completion_code);

    if completion_code == 0 {
        // success — check whether this is an Enable Slot completion (slot_id != 0)
        if slot_id != 0 {
            println!("[xHCI] Enable Slot succeeded for slot {} on controller {:#x}", slot_id, bar_phys);
            on_enable_slot_success(bar_phys, slot_id);
        } else {
            println!("[xHCI] Command completed successfully but slot_id==0 — unknown command");
        }
    } else {
        println!("[xHCI] Command Completion reported error code {}", completion_code);
    }
}

/// Placeholder invoked when Enable Slot completes successfully. Will perform next steps:
/// allocate Device Context Base Address Array (DCBAA) and submit Address Device command.
fn on_enable_slot_success(_bar_phys: u64, slot_id: u8) {
    println!("[xHCI] on_enable_slot_success: slot={} (placeholder) — will implement DCBAA + Address Device next", slot_id);
    // TODO: allocate DCBAA, prepare device context, issue Address Device command via ASQ
}

/// Prepare (diagnostic) writes for runtime ERST/ERDP and doorbell addresses.
/// This function currently does not perform MMIO writes — it only computes and
/// logs the addresses/values we would write. After validating runtime base
/// from serial logs we can enable actual writes.
pub fn prepare_runtime_erst_and_doorbell() {
    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        if let Some(rt) = c.cap_rt_off {
            let runtime_base = (c.op_base_virt as u64).wrapping_sub(0) + (rt as u64);
            println!("[xHCI] prepare_runtime: controller {:#x} runtime_base={:#x}", c.bar_phys, runtime_base);

            // Planned ERSTSZ (number of segments - 1). We allocated one page for ERST,
            // and will use a single segment for the event ring. ERSTSZ = segments - 1 => 0
            let erstsz: u32 = 0;

            // ERSTBA: physical base of the ERST table (we stored erst_phys)
            if let Some(erst_phys) = c.erst_phys {
                let erst_lo = erst_phys as u32;
                let erst_hi = (erst_phys as u64 >> 32) as u32;
                println!("[xHCI] would write ERSTSZ={:#x} to runtime+ERSTSZ", erstsz);
                println!("[xHCI] would write ERSTBA_LO={:#x} ERSTBA_HI={:#x}", erst_lo, erst_hi);
            } else {
                println!("[xHCI] no ERST phys allocated for controller {:#x}", c.bar_phys);
            }

            // ERDP: Event Ring Dequeue Pointer (we'll point to start of event ring virt)
            if let Some(ev_phys) = c.event_ring_phys {
                let ev_virt = c.event_ring_virt.unwrap_or(0);
                println!("[xHCI] would set ERDP to event_ring_virt={:#x}", ev_virt);
            } else {
                println!("[xHCI] no event ring allocated for controller {:#x}", c.bar_phys);
            }

            // Doorbell: compute candidate address for doorbell registers.
            // Doorbell array generally follows capability+operational regions; we compute a candidate
            // using op_base + 0x1000 as a conservative guess and log it for diagnostics.
            let doorbell_candidate = c.op_base_virt + 0x1000;
            println!("[xHCI] doorbell candidate @ {:#x}", doorbell_candidate);
        } else {
            println!("[xHCI] controller {:#x} has no cap_rt_off; cannot prepare runtime writes", c.bar_phys);
        }
    }
}

/// Perform actual MMIO writes to runtime registers: ERSTSZ, ERSTBA (lo/hi), and ERDP.
/// Writes are gated by `RUNTIME_WRITES_ENABLED`. Logs actions either way.
pub fn write_runtime_erst() {
    const ERSTSZ_OFF: u64 = 0x18; // chosen candidate offsets relative to runtime base
    const ERSTBA_LO_OFF: u64 = 0x20;
    const ERSTBA_HI_OFF: u64 = 0x24;
    const ERDP_OFF: u64 = 0x28;

    let list = XHCI_CONTROLLERS.lock();
    for c in list.iter() {
        if let Some(rt) = c.cap_rt_off {
            let runtime_base = c.mmio_base_virt + (rt as u64);
            println!("[xHCI] write_runtime_erst: controller {:#x} runtime_base={:#x}", c.bar_phys, runtime_base);

            if c.erst_phys.is_none() || c.event_ring_phys.is_none() {
                println!("[xHCI] controller {:#x} missing ERST or event ring; skip", c.bar_phys);
                continue;
            }

            let erst_phys = c.erst_phys.unwrap() as u64;
            let ev_ring_phys = c.event_ring_phys.unwrap() as u64;

            // ERSTSZ = segments - 1 => 0 for 1 segment
            let erstsz: u32 = 0;
            println!("[xHCI] would write ERSTSZ={:#x} ERSTBA={:#x} ERDP={:#x}", erstsz, erst_phys, ev_ring_phys);

            if RUNTIME_WRITES_ENABLED.load(Ordering::Relaxed) {
                unsafe {
                    let erstsz_ptr = (runtime_base + ERSTSZ_OFF) as *mut u32;
                    core::ptr::write_volatile(erstsz_ptr, erstsz);

                    let erstba_lo_ptr = (runtime_base + ERSTBA_LO_OFF) as *mut u32;
                    let erstba_hi_ptr = (runtime_base + ERSTBA_HI_OFF) as *mut u32;
                    core::ptr::write_volatile(erstba_lo_ptr, erst_phys as u32);
                    core::ptr::write_volatile(erstba_hi_ptr, (erst_phys >> 32) as u32);

                    let erdp_ptr = (runtime_base + ERDP_OFF) as *mut u64;
                    core::ptr::write_volatile(erdp_ptr, ev_ring_phys);
                }
                println!("[xHCI] runtime ERST/ERDP written for controller {:#x}", c.bar_phys);
            } else {
                println!("[xHCI] RUNTIME_WRITES_ENABLED disabled; not writing ERST/ERDP for {:#x}", c.bar_phys);
            }
        } else {
            println!("[xHCI] controller {:#x} has no RTSOFF; skip runtime write", c.bar_phys);
        }
    }
}
