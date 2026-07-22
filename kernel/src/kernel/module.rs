use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub type ModuleId = u64;

/// Состояние модуля
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModuleState {
    Unloaded,
    Loaded,
    Initialized,
    Running,
    Stopped,
    Error,
}

/// Трейт модуля ядра
pub trait Module: Send + Sync {
    fn id(&self) -> ModuleId;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn author(&self) -> &str;
    fn description(&self) -> &str;
    
    fn init(&mut self) -> Result<(), &'static str> { Ok(()) }
    fn start(&mut self) -> Result<(), &'static str> { Ok(()) }
    fn stop(&mut self) -> Result<(), &'static str> { Ok(()) }
    fn cleanup(&mut self) -> Result<(), &'static str> { Ok(()) }
}

pub struct ModuleInfo {
    pub id: ModuleId,
    pub name: String,
    pub version: String,
    pub author: String,
    pub state: ModuleState,
}

static MODULES: Mutex<Vec<ModuleInfo>> = Mutex::new(Vec::new());

pub fn init() {
    crate::println!("[KERNEL] Module subsystem initialized");
    
    // Регистрируем встроенные модули (built-in)
    register_builtin("vfs", "0.1.0", "OneX01", "Virtual File System");
    register_builtin("console", "0.1.0", "OneX01", "Console Manager");
    register_builtin("scheduler", "0.1.0", "OneX01", "Task Scheduler (stub)");
}

fn register_builtin(name: &str, version: &str, author: &str, _desc: &str) {
    let id = MODULES.lock().len() as ModuleId + 1;
    let info = ModuleInfo {
        id,
        name: String::from(name),
        version: String::from(version),
        author: String::from(author),
        state: ModuleState::Initialized,
    };
    MODULES.lock().push(info);
}

pub fn shutdown_all() {
    crate::println!("[KERNEL] Stopping all modules...");
}

pub fn list() -> Vec<ModuleInfo> {
    Vec::new() // TODO
}

pub fn count() -> usize {
    MODULES.lock().len()
}