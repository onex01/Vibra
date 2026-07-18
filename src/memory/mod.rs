pub mod pmm;
pub mod heap;
pub mod paging;

// Убрали pub use pmm::*, так как он не использовался здесь напрямую

pub fn init(memory_map: &[&limine::memmap::Entry], hhdm_offset: u64) {
    crate::println!("[MEM] Initializing Physical Memory Manager...");
    pmm::init(memory_map);
    paging::init(hhdm_offset);

    unsafe {
        heap::init(hhdm_offset);
    }
}
