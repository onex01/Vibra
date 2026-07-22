use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW};

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    // Текущий активный формат
    let format = crate::display::get_active_format();
    let count = crate::display::backend_count();

    console.print_colored("=== Дисплей ===\n", COLOR_CYAN);
    console.print("  Текущий формат: ");
    console.print_colored(format.name(), COLOR_GREEN);
    console.print("\n");
    console.print("  Бэкендов: ");
    console.print_num(count);
    console.print("\n\n");

    // Список всех бэкендов
    if count > 0 {
        console.print_colored("  Доступные бэкенды:\n", COLOR_CYAN);
        for i in 0..count {
            if let Some((name, w, h, bpp, fmt)) = crate::display::get_backend_info(i) {
                console.print("    [");
                console.print_num(i);
                console.print("] ");
                console.print_colored(&name, COLOR_YELLOW);
                console.print("  ");
                console.print_num(w as usize);
                console.print("x");
                console.print_num(h as usize);
                console.print("  ");
                console.print_num(bpp as usize);
                console.print("bpp  ");
                console.print(fmt.name());
                console.print("\n");
            }
        }
    } else {
        console.print_colored("  Нет зарегистрированных бэкендов\n", COLOR_YELLOW);
    }

    // Информация о framebuffer консоли
    console.print("\n");
    console.print_colored("  Консоль:\n", COLOR_CYAN);
    console.print("    Экран: ");
    console.print_num(console.fb_width());
    console.print("x");
    console.print_num(console.fb_height());
    console.print("\n");
    console.print("    Питч: ");
    console.print_num(console.fb_pitch());
    console.print(" слов\n");

    CmdResult::Ok
}
