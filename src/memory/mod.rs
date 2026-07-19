pub mod pmm;
pub mod heap;
pub mod paging;
pub mod vmm;

pub fn init(memory_map: &[&limine::memmap::Entry], hhdm_offset: u64) {
    crate::println!("[MEM] Initializing Physical Memory Manager...");
    pmm::init(memory_map);
    paging::init(hhdm_offset);

    unsafe {
        heap::init(hhdm_offset);
    }
}
