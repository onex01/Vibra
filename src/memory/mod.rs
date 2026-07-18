pub mod pmm;
pub mod heap;

// Убрали pub use pmm::*, так как он не использовался здесь напрямую

pub fn init(memory_map: &[&limine::memmap::Entry]) {
    crate::println!("[MEM] Initializing Physical Memory Manager...");
    pmm::init(memory_map);
    
    unsafe {
        heap::init();
    }
}