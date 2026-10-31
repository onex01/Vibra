pub mod pmm;
pub mod heap;
pub mod dma;
pub mod paging;
pub mod vmm;

use alloc::string::String;

/// Сводка памяти в одном месте.
///
/// `used` — всё, что не свободно по PMM (ядро, page tables, reserved), а не
/// «сумма кучи»: раньше `free` и `top` считали это по-разному и показывали
/// разные числа для одной и той же машины.
#[derive(Clone, Copy, Debug)]
pub struct MemInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub heap_used: u64,
    pub heap_total: u64,
}

pub fn meminfo() -> MemInfo {
    let (used_frames, total_frames) = pmm::stats();
    let (heap_used, heap_total) = heap::stats();
    let frame = pmm::FRAME_SIZE as u64;
    let total_bytes = total_frames as u64 * frame;
    let used_bytes = used_frames as u64 * frame;
    MemInfo {
        total_bytes,
        used_bytes,
        free_bytes: total_bytes.saturating_sub(used_bytes),
        heap_used: heap_used as u64,
        heap_total: heap_total as u64,
    }
}

impl MemInfo {
    pub fn used_percent(&self) -> u32 {
        if self.total_bytes == 0 {
            0
        } else {
            ((self.used_bytes * 100) / self.total_bytes) as u32
        }
    }
}

/// Человекочитаемый размер: B / KiB / MiB / GiB.
///
/// Один формат для `free`, `top`, `neofetch` и `/proc/meminfo`, чтобы не
/// держать по своей арифметике в каждой команде.
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        let whole = bytes / GIB;
        let frac = (bytes % GIB) * 10 / GIB;
        alloc::format!("{}.{} GiB", whole, frac)
    } else if bytes >= MIB {
        let whole = bytes / MIB;
        let frac = (bytes % MIB) * 10 / MIB;
        alloc::format!("{}.{} MiB", whole, frac)
    } else if bytes >= KIB {
        alloc::format!("{} KiB", bytes / KIB)
    } else {
        alloc::format!("{} B", bytes)
    }
}

/// Компактная форма для строки статуса: `3.4/16.0 GiB`.
pub fn format_pair(used: u64, total: u64) -> String {
    alloc::format!("{}/{}", format_bytes(used), format_bytes(total))
}

pub fn init(memory_map: &[&limine::memmap::Entry], hhdm_offset: u64) {
    crate::println!("[MEM] Initializing Physical Memory Manager...");
    pmm::init(memory_map);
    paging::init(hhdm_offset);

    unsafe {
        heap::init(hhdm_offset);
    }
}
