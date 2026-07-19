// Virtual Devices — виртуальные устройства для тестирования и эмуляции.
//
// VirtIO Block — базовый блочный драйвер для QEMU VirtIO
// VirtIO Net — заглушка для сетевого устройства
// PC Speaker — эмуляция динамика через PIT

pub mod virtio_block;
pub mod virtio_net;
pub mod pc_speaker;

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

/// Инициализация подсистемы виртуальных устройств
pub fn init() {
    crate::println!("[DEVICES] Virtual device subsystem initialized");

    // Регистрируем базовые виртуальные устройства
    register_device("virtio-block-0", VirtDeviceType::Block);
    register_device("virtio-net-0", VirtDeviceType::Network);
    register_device("pc-speaker", VirtDeviceType::Console);
    register_device("ps2-keyboard", VirtDeviceType::Input);
    register_device("ps2-mouse", VirtDeviceType::Input);
}
