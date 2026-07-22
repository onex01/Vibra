pub mod device;
pub mod driver;
pub mod module;
pub mod event;
pub mod registry;

use crate::println;
use crate::version;

/// Инициализация ядра
pub fn init() {
    println!("========================================");
    println!("  Vibra Kernel v{} \"{}\"", version::KERNEL_VERSION, version::KERNEL_CODENAME);
    println!("========================================");
    
    // Инициализация подсистем ядра
    registry::init();
    event::init();
    device::init();
    driver::init();
    module::init();
    
    println!("[KERNEL] All subsystems initialized");
    println!("[KERNEL] Device registry: {} devices", registry::device_count());
    println!("[KERNEL] Driver registry: {} drivers", registry::driver_count());
}

/// Выключение ядра (graceful shutdown)
pub fn shutdown() {
    println!("[KERNEL] Shutting down...");
    module::shutdown_all();
    device::shutdown_all();
    println!("[KERNEL] Shutdown complete");
}