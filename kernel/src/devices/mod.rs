// Virtual Devices — реально обнаруженные устройства ввода и вспомогательные
// устройства (PC speaker). Раньше здесь регистрировались фиктивные
// "virtio-block-0"/"virtio-net-0" независимо от наличия оборудования.

pub mod virtio_block;
pub mod pc_speaker;
pub mod ps2_mouse;

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Тип виртуального устройства
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VirtDeviceType {
    Block,
    Network,
    Console,
    Input,
    GPU,
}

/// Состояние виртуального устройства
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VirtDeviceState {
    Reset,
    Ready,
    Running,
    Error,
}

/// Трейт виртуального устройства
pub trait VirtDevice: Send + Sync {
    fn name(&self) -> &str;
    fn device_type(&self) -> VirtDeviceType;
    fn state(&self) -> VirtDeviceState;
    fn init(&mut self) -> Result<(), &'static str> { Ok(()) }
    fn reset(&mut self) -> Result<(), &'static str> { Ok(()) }
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str> { Err("not supported") }
    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize, &'static str> { Err("not supported") }
}

/// Информация о зарегистрированном устройстве
pub struct VirtDeviceInfo {
    pub name: String,
    pub device_type: VirtDeviceType,
    pub state: VirtDeviceState,
}

static DEVICES: Mutex<Vec<VirtDeviceInfo>> = Mutex::new(Vec::new());

/// Регистрация виртуального устройства
pub fn register_device(name: &str, device_type: VirtDeviceType) {
    let info = VirtDeviceInfo {
        name: String::from(name),
        device_type,
        state: VirtDeviceState::Reset,
    };
    DEVICES.lock().push(info);
    crate::println!("[DEVICES] Registered virtual device: {} ({:?})", name, device_type);
}

/// Количество зарегистрированных устройств
pub fn count() -> usize {
    DEVICES.lock().len()
}

/// Инициализация подсистемы устройств.
pub fn init() {
    crate::println!("[DEVICES] Input device subsystem initialized");

    // Регистрируем только то, что реально ответило на probe.
    if crate::keyboard::status().present {
        register_device("ps2-keyboard", VirtDeviceType::Input);
    }

    if ps2_mouse::init() {
        register_device("ps2-mouse", VirtDeviceType::Input);
    }
}
