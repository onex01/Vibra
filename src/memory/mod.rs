pub mod pmm;

pub fn init(memory_map: &[&limine::memmap::Entry]) {
    crate::println!("[MEM] Initializing Physical Memory Manager...");
    pmm::init(memory_map);
}