use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;
use super::device::{DeviceId, DeviceType};

/// Идентификатор драйвера
pub type DriverId = u64;

/// Трейт драйвера — управляет одним или несколькими устройствами
pub trait Driver: Send + Sync {
    /// Уникальный ID драйвера
    fn id(&self) -> DriverId;
    
    /// Имя драйвера
    fn name(&self) -> &str;
    
    /// Версия драйвера
    fn version(&self) -> &str;
    
    /// Какие типы устройств поддерживает
    fn supported_types(&self) -> &[DeviceType];
    
    /// Инициализация драйвера
    fn init(&mut self) -> Result<(), &'static str> { Ok(()) }
    
    /// Выключение драйвера
    fn shutdown(&mut self) -> Result<(), &'static str> { Ok(()) }
    
    /// Привязка к устройству
    fn bind(&mut self, _device_id: DeviceId) -> Result<(), &'static str> {
        Err("bind not supported")
    }
    
    /// Отвязка от устройства
    fn unbind(&mut self, _device_id: DeviceId) -> Result<(), &'static str> {
        Err("unbind not supported")
    }
}

/// Информация о драйвере
pub struct DriverInfo {
    pub id: DriverId,
    pub name: String,
    pub version: String,
    pub supported_types: Vec<DeviceType>,
    pub bound_devices: Vec<DeviceId>,
}

static DRIVERS: Mutex<Vec<DriverInfo>> = Mutex::new(Vec::new());
static mut NEXT_DRIVER_ID: DriverId = 1;

pub fn init() {
    crate::println!("[KERNEL] Driver subsystem initialized");
}

pub fn register(name: &str, version: &str, types: &[DeviceType]) -> DriverId {
    unsafe {
        let id = NEXT_DRIVER_ID;
        NEXT_DRIVER_ID += 1;
        
        let info = DriverInfo {
            id,
            name: String::from(name),
            version: String::from(version),
            supported_types: types.to_vec(),
            bound_devices: Vec::new(),
        };
        
        DRIVERS.lock().push(info);
        crate::println!("[KERNEL] Registered driver: {} v{} (id={})", name, version, id);
        id
    }
}

pub fn count() -> usize {
    DRIVERS.lock().len()
}