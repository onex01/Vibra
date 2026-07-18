use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Типы системных событий
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    DeviceAdded,
    DeviceRemoved,
    DriverLoaded,
    ModuleLoaded,
    KeyPressed,
    TimerTick,
    FileSystemChanged,
    Custom(u32),
}

/// Системное событие
pub struct Event {
    pub event_type: EventType,
    pub source: String,
    pub data: usize,
}

/// Подписчик на события
pub type EventHandler = fn(&Event);

struct Subscription {
    event_type: EventType,
    handler: EventHandler,
}

static SUBSCRIPTIONS: Mutex<Vec<Subscription>> = Mutex::new(Vec::new());

pub fn init() {
    crate::println!("[KERNEL] Event bus initialized");
}

/// Подписка на событие
pub fn subscribe(event_type: EventType, handler: EventHandler) {
    SUBSCRIPTIONS.lock().push(Subscription { event_type, handler });
}

/// Публикация события
pub fn publish(event: Event) {
    let subs = SUBSCRIPTIONS.lock();
    for sub in subs.iter() {
        if sub.event_type == event.event_type {
            (sub.handler)(&event);
        }
    }
}