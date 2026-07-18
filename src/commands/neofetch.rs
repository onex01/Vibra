// neofetch — системная сводка в стиле neofetch/screenfetch.
//
// Комбинирует информацию из version, sysinfo, about, uptime, logo
// в один компактный и красивый вывод.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_YELLOW,
                         COLOR_VIBRA_PROMPT, COLOR_VIBRA_FG, COLOR_DARK_GRAY};
use crate::version;
use crate::interrupts;
use crate::kernel;
use alloc::string::String;
use alloc::format;

// Линии ASCII-логотипа (8 строк). Каждая строка — один ряд справа от логотипа.
const LOGO: [&str; 8] = [
    "     __     ___ _               ",
    "     \\ \\   / (_) |__  _ __ __ _ ",
    "      \\ \\ / /| | '_ \\| '__/ _` |",
    "       \\ V / | | |_) | | | (_| |",
    "        \\_/  |_|_.__/|_|  \\__,_|",
    "",
    "   Modular monolithic kernel",
    "   written in Rust",
];

// Информационные строки (по одной на каждую строку логотипа).
// Спец-символы: {C} = цвет C, {} = сброс на VIBRA_FG.
fn info_lines() -> alloc::vec::Vec<String> {
    let ticks = interrupts::idt::ticks();
    let seconds = ticks / interrupts::idt::TIMER_HZ;
    let minutes = seconds / 60;
    let hours = minutes / 60;

    let uptime_str = format!("{}h {}m {}s", hours, minutes % 60, seconds % 60);

    let (heap_used, heap_total) = crate::memory::heap::stats();
    let heap_pct = if heap_total != 0 { (heap_used * 100) / heap_total } else { 0 };

    let (pmm_used, pmm_total) = crate::memory::pmm::stats();
    let mem_mb = (pmm_total * crate::memory::pmm::FRAME_SIZE) / (1024 * 1024);
    let used_mb = (pmm_used * crate::memory::pmm::FRAME_SIZE) / (1024 * 1024);
    let free_mb = mem_mb - used_mb;

    // Для строк с несколькими цветными секциями строим String вручную,
    // чтобы спец-коды не конфликтовали с {}-плейсхолдерами format!.
    let devices = format!("{}", kernel::registry::device_count());
    let drivers = format!("{}", kernel::registry::driver_count());
    let modules = format!("{}", kernel::registry::module_count());

    let dev_line = {
        let mut s = String::new();
        s.push_str(CYL); s.push_str(BOLD); s.push_str("Devices"); s.push_str(RST);
        s.push_str("    ");
        s.push_str(&devices);
        s.push_str("  ");
        s.push_str(SEP); s.push_str(CYL); s.push_str(BOLD); s.push_str("Drivers"); s.push_str(RST);
        s.push_str("  ");
        s.push_str(&drivers);
        s.push_str("  ");
        s.push_str(SEP); s.push_str(CYL); s.push_str(BOLD); s.push_str("Modules"); s.push_str(RST);
        s.push_str("  ");
        s.push_str(&modules);
        s
    };

    let author_line = {
        let mut s = String::new();
        s.push_str(CYL); s.push_str(BOLD); s.push_str("Author"); s.push_str(RST);
        s.push_str("     ");
        s.push_str(YEL); s.push_str(version::AUTHOR); s.push_str(RST);
        s.push_str("  ");
        s.push_str(SEP); s.push_str(CYL); s.push_str(BOLD); s.push_str("License"); s.push_str(RST);
        s.push_str("  ");
        s.push_str(version::LICENSE);
        s
    };

    alloc::vec![
        // Простые строки — format! безопасен (нет вложенных {})
        format!("{}{}OS{}        Vibra OS v{} \"{}\"", CYL, BOLD, RST, version::OS_VERSION, version::OS_CODENAME),
        format!("{}{}Kernel{}    v{} \"{}\"", CYL, BOLD, RST, version::KERNEL_VERSION, version::KERNEL_CODENAME),
        format!("{}{}Arch{}       {}", CYL, BOLD, RST, version::ARCHITECTURE),
        format!("{}{}Uptime{}     {}", CYL, BOLD, RST, uptime_str),
        format!("{}{}Memory{}     {} MB / {} MB ({} free)", CYL, BOLD, RST, used_mb, mem_mb, free_mb),
        format!("{}{}Heap{}       {}/{} KB ({}% used)", CYL, BOLD, RST, heap_used / 1024, heap_total / 1024, heap_pct),
        dev_line,
        author_line,
    ]
}

// Цвет-маркеры, которые мы раскрашиваем через Console.
// Строки содержат спец-коды: {CYL}{BOLD}label{RST} value.
const CYL: &str = "\x01"; // → COLOR_CYAN
const BOLD: &str = "\x02"; // → COLOR_VIBRA_PROMPT (яркий зелёный вместо bold)
const YEL: &str = "\x03";  // → COLOR_YELLOW
const RST: &str = "\x04";  // → COLOR_VIBRA_FG
const SEP: &str = "\x04";  // разделитель — сброс цвета

fn print_rich(console: &mut Console, s: &str) {
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\x01' => console.set_fg(COLOR_CYAN),
            '\x02' => console.set_fg(COLOR_VIBRA_PROMPT),
            '\x03' => console.set_fg(COLOR_YELLOW),
            '\x04' => console.set_fg(COLOR_VIBRA_FG),
            other => console.print_char(other),
        }
    }
}

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    console.print("\n");

    let info = info_lines();

    for i in 0..LOGO.len() {
        // Логотип — cyan
        console.print_colored(LOGO[i], COLOR_CYAN);

        // Разделитель: если строка логотипа не пуста, ставим " | "
        if !LOGO[i].is_empty() && i < info.len() {
            console.print_colored("  │ ", COLOR_DARK_GRAY);
            print_rich(console, &info[i]);
        } else if i < info.len() {
            console.print("    ");
            print_rich(console, &info[i]);
        }
        console.print("\n");
    }

    // Разделительная линия снизу
    console.print_colored("  ───────────────────────────────────────────\n", COLOR_DARK_GRAY);
    console.print_colored("  Shell: built-in with tab-completion & history\n", COLOR_VIBRA_FG);
    console.print_colored("  Boot:  Limine (UEFI) → QEMU q35\n", COLOR_VIBRA_FG);
    console.print("\n");

    // Восстанавливаем дефолтный цвет
    console.set_fg(COLOR_VIBRA_FG);

    CmdResult::Ok
}
