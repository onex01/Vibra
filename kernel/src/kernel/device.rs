use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Идентификатор устройства
pub type DeviceId = u64;

/// Тип устройства
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceType {
    Console,
    Keyboard,
    Mouse,
    Disk,
    Network,
    Display,
    Audio,
    Timer,
    Bus,
    Unknown,
}

/// Состояние устройства
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceState {
    Uninitialized,
    Ready,
    Active,
    Error,
    Suspended,
}

/// Трейт устройства — базовая абстракция для любого hardware/software устройства
pub trait Device: Send + Sync {
    /// Уникальный ID устройства
    fn id(&self) -> DeviceId;
    
    /// Имя устройства
    fn name(&self) -> &str;
    
    /// Тип устройства
    fn device_type(&self) -> DeviceType;
    
    /// Текущее состояние
    fn state(&self) -> DeviceState;
    
    /// Инициализация устройства
    fn init(&mut self) -> Result<(), &'static str> { Ok(()) }
    
    /// Выключение устройства
    fn shutdown(&mut self) -> Result<(), &'static str> { Ok(()) }
    
    /// Чтение данных (если применимо)
    fn read(&self, _buf: &mut [u8]) -> Result<usize, &'static str> {
        Err("read not supported")
    }
    
    /// Запись данных (если применимо)
    fn write(&mut self, _buf: &[u8]) -> Result<usize, &'static str> {
        Err("write not supported")
    }
    
    /// Управление устройством (ioctl-подобные команды)
    fn control(&mut self, _cmd: u32, _arg: usize) -> Result<usize, &'static str> {
        Err("control not supported")
    }
}

/// Информация об устройстве для реестра
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub device_type: DeviceType,
    pub state: DeviceState,
}

// Глобальный список зарегистрированных устройств
static DEVICES: Mutex<Vec<DeviceInfo>> = Mutex::new(Vec::new());
static mut NEXT_DEVICE_ID: DeviceId = 1;

/// Инициализация подсистемы устройств
pub fn init() {
    crate::println!("[KERNEL] Device subsystem initialized");
}

/// Регистрация нового устройства
pub fn register(name: &str, device_type: DeviceType) -> DeviceId {
    unsafe {
        let id = NEXT_DEVICE_ID;
        NEXT_DEVICE_ID += 1;
        
        let info = DeviceInfo {
            id,
            name: String::from(name),
            device_type,
            state: DeviceState::Uninitialized,
        };
        
        DEVICES.lock().push(info);
        crate::println!("[KERNEL] Registered device: {} (id={}, type={:?})", name, id, device_type);
        id
    }
}

/// Список всех устройств
pub fn list() -> Vec<DeviceInfo> {
    // Возвращаем копию (DeviceInfo не Clone из-за String, поэтому упростим)
    Vec::new() // TODO: вернуть реальный список
}

/// Количество устройств
pub fn count() -> usize {
    DEVICES.lock().len()
}

/// Выключение всех устройств
pub fn shutdown_all() {
    crate::println!("[KERNEL] Shutting down all devices...");
}