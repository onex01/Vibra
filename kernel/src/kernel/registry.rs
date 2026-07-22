use super::{device, driver, module};

/// Единая точка доступа ко всем ресурсам ядра
pub fn init() {
    crate::println!("[KERNEL] Resource registry initialized");
}

pub fn device_count() -> usize {
    device::count()
}

pub fn driver_count() -> usize {
    driver::count()
}

pub fn module_count() -> usize {
    module::count()
}