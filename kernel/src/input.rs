// Unified Input Subsystem — унифицированные структуры событий ввода.
//
// Поддерживает: клавиатура (Key), мышь (MouseMove, MouseClick).
// В будущем: touch, gamepad.

use spin::Mutex;

/// Модификаторы клавиатуры
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps_lock: bool,
}

/// Событие клавиатуры
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: KeyModifiers,
    pub pressed: bool, // true = нажатие, false = отпускание
}

/// Клавиша (расширенная версия keyboard::Key)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Space,
    // Стрелки
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    // Функциональные
    F(u8),
    // Цифры верхнего ряда
    Num(u8),
    // Специальные
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Unknown(u8),
}

/// Событие мыши
#[derive(Debug, Clone, Copy)]
pub struct MouseMoveEvent {
    pub dx: i16,
    pub dy: i16,
}

/// Кнопка мыши
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Событие клика мыши
#[derive(Debug, Clone, Copy)]
pub struct MouseClickEvent {
    pub button: MouseButton,
    pub pressed: bool,
    pub x: i16,
    pub y: i16,
}

/// Все типы событий ввода
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    Key(KeyEvent),
    MouseMove(MouseMoveEvent),
    MouseClick(MouseClickEvent),
}

/// Обработчик событий ввода
pub type InputHandler = fn(&InputEvent);

struct InputState {
    handlers: [Option<InputHandler>; 16],
    handler_count: usize,
    modifiers: KeyModifiers,
}

static INPUT_STATE: Mutex<InputState> = Mutex::new(InputState {
    handlers: [None; 16],
    handler_count: 0,
    modifiers: KeyModifiers {
        shift: false,
        ctrl: false,
        alt: false,
        caps_lock: false,
    },
});

/// Подписаться на события ввода
pub fn subscribe(handler: InputHandler) {
    let mut state = INPUT_STATE.lock();
    if state.handler_count < 16 {
        let idx = state.handler_count;
        state.handlers[idx] = Some(handler);
        state.handler_count = idx + 1;
    }
}

/// Отправить событие клавиатуры (вызывается из keyboard драйвера)
pub fn fire_key_event(key: Key, pressed: bool) {
    let mut state = INPUT_STATE.lock();

    // Обновляем модификаторы
    match key {
        Key::Char(c) if c == '\x00' => {} // ignore
        _ => {}
    }

    let event = InputEvent::Key(KeyEvent {
        key,
        modifiers: state.modifiers,
        pressed,
    });

    for i in 0..state.handler_count {
        if let Some(handler) = state.handlers[i] {
            handler(&event);
        }
    }
}

/// Отправить событие движения мыши
pub fn fire_mouse_move(dx: i16, dy: i16) {
    let state = INPUT_STATE.lock();
    let event = InputEvent::MouseMove(MouseMoveEvent { dx, dy });
    for i in 0..state.handler_count {
        if let Some(handler) = state.handlers[i] {
            handler(&event);
        }
    }
}

/// Отправить событие клика мыши
pub fn fire_mouse_click(button: MouseButton, pressed: bool, x: i16, y: i16) {
    let state = INPUT_STATE.lock();
    let event = InputEvent::MouseClick(MouseClickEvent { button, pressed, x, y });
    for i in 0..state.handler_count {
        if let Some(handler) = state.handlers[i] {
            handler(&event);
        }
    }
}

/// Инициализация подсистемы ввода
pub fn init() {
    crate::println!("[INPUT] Input subsystem initialized");
}
