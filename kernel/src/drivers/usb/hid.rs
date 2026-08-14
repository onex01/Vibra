use crate::println;
use crate::keyboard::Key as KKey;

/// Минимальный skeleton USB HID support.
/// Пока что не реализована полноценно USB-энумерация — создаём инфраструктуру
/// и (при наличии контроллеров) делаем тестовую инъекцию клавиш в keyboard.
pub fn init() {
    println!("[HID] Initializing HID subsystem (minimal)");
    // Если нужно, можно здесь попытаться провести enumeration через xHCI.
    // Для быстрых проверок вставим тестовую строку в очередь клавиатуры,
    // чтобы убедиться, что shell принимает внешние события.
    let test = "hid_ok\n";
    for &b in test.as_bytes() {
        let key = match b {
            b'\n' => KKey::Enter,
            b' ' => KKey::Char(' '),
            _ if (0x20..=0x7E).contains(&b) => KKey::Char(b as char),
            _ => continue,
        };
        crate::keyboard::inject_key(key);
    }
    println!("[HID] Test keys injected (hid_ok)");
}

/// Placeholder for future HID event handling
pub fn handle_hid_event() {
    // TODO: parse HID reports and inject via `crate::keyboard::inject_key`.
}
