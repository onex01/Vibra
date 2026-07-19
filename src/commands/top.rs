use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_YELLOW, COLOR_CYAN, COLOR_RED, COLOR_WHITE};
use crate::fs;
use alloc::string::String;
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let show_once = args.iter().any(|a| *a == "-b" || *a == "--batch");

    if show_once {
        print_stats(console);
    } else {
        // Одноразовый вывод (в будущем — обновление по таймеру)
        print_stats(console);
    }

    CmdResult::Ok
}

fn print_stats(console: &mut Console) {
    // === CPU ===
    print_cpu_info(console);

    // === Память ===
    print_memory_info(console);

    // === Процессы ===
    print_processes(console);

    // === Файловая система ===
    print_fs_info(console);
}

fn print_cpu_info(console: &mut Console) {
    console.print_colored("┌─── CPU ──────────────────────────────────────────────────────┐\n", COLOR_YELLOW);

    let ticks = crate::interrupts::idt::ticks();
    let uptime_secs = ticks / 100; // PIT 100 Hz
    let uptime_mins = uptime_secs / 60;
    let uptime_hours = uptime_mins / 60;

    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Uptime: ", COLOR_WHITE);
    console.print_num(uptime_hours as usize);
    console.print("h ");
    console.print_num((uptime_mins % 60) as usize);
    console.print("m ");
    console.print_num((uptime_secs % 60) as usize);
    console.print("s");
    console.print_colored("                              │\n", COLOR_YELLOW);

    // Оценка загрузки CPU на основе тиков
    // Пока просто показываем что таймер работает
    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Timer ticks: ", COLOR_WHITE);
    console.print_num(ticks as usize);
    console.print_colored(" (100 Hz)                          │\n", COLOR_YELLOW);

    // Показываем "загрузку" — на основе количества тиков относительно uptime
    let load_pct = if uptime_secs > 0 {
        // Простая эвристика: если тики идут нормально — CPU загружен
        // В реальной ОС это считалось бы по idle тикам
        let expected_ticks = uptime_secs * 100;
        if ticks > expected_ticks {
            100
        } else {
            (ticks * 100) / expected_ticks.max(1)
        }
    } else {
        0
    };

    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Load:      ", COLOR_WHITE);
    draw_bar(console, load_pct as usize, 30);
    console.print(" ");
    console.print_num(load_pct as usize);
    console.print("%");
    console.put_char('\n');

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_YELLOW);
    console.put_char('\n');
}

fn print_memory_info(console: &mut Console) {
    console.print_colored("┌─── Memory ───────────────────────────────────────────────────┐\n", COLOR_GREEN);

    // PMM stats
    let (used_frames, total_frames) = crate::memory::pmm::stats();
    let total_bytes = total_frames * 4096;
    let used_bytes = used_frames * 4096;
    let free_bytes = total_bytes - used_bytes;

    let total_kb = total_bytes / 1024;
    let used_kb = used_bytes / 1024;
    let free_kb = free_bytes / 1024;
    let total_mb = total_bytes / (1024 * 1024);
    let used_mb = used_bytes / (1024 * 1024);
    let free_mb = free_bytes / (1024 * 1024);

    let mem_pct = if total_bytes > 0 { (used_bytes * 100) / total_bytes } else { 0 };

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Physical RAM:  ", COLOR_WHITE);
    console.print_num(used_mb);
    console.print(" / ");
    console.print_num(total_mb);
    console.print(" MB  (");
    console.print_num(used_kb);
    console.print(" / ");
    console.print_num(total_kb);
    console.print(" KB)");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Used:         ", COLOR_WHITE);
    draw_bar(console, mem_pct, 30);
    console.print(" ");
    console.print_num(mem_pct);
    console.print("%");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Free:         ", COLOR_WHITE);
    console.print_num(free_mb);
    console.print(" MB (");
    console.print_num(free_kb);
    console.print(" KB)");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Frames:       ", COLOR_WHITE);
    console.print_num(used_frames);
    console.print(" used / ");
    console.print_num(total_frames);
    console.print(" total");
    console.put_char('\n');

    // Heap stats
    let (heap_used, heap_total) = crate::memory::heap::stats();
    let heap_used_kb = heap_used / 1024;
    let heap_total_kb = heap_total / 1024;
    let heap_pct = if heap_total > 0 { (heap_used * 100) / heap_total } else { 0 };

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Heap:         ", COLOR_WHITE);
    console.print_num(heap_used_kb);
    console.print(" / ");
    console.print_num(heap_total_kb);
    console.print(" KB");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Heap usage:   ", COLOR_WHITE);
    draw_bar(console, heap_pct, 30);
    console.print(" ");
    console.print_num(heap_pct);
    console.print("%");
    console.put_char('\n');

    // Строковое представление
    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Usage:        ", COLOR_WHITE);
    console.print("used=");
    console.print_num(used_mb);
    console.print("MB free=");
    console.print_num(free_mb);
    console.print("MB total=");
    console.print_num(total_mb);
    console.print("MB");
    console.put_char('\n');

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_GREEN);
    console.put_char('\n');
}

fn print_processes(console: &mut Console) {
    console.print_colored("┌─── Processes ────────────────────────────────────────────────┐\n", COLOR_CYAN);

    let task_count = crate::task::task_count();

    console.print_colored("│ ", COLOR_CYAN);
    console.print_colored("Tasks: ", COLOR_WHITE);
    console.print_num(task_count);
    console.print(" total");
    console.put_char('\n');

    // Показываем информацию о текущих задачах
    console.print_colored("│ ", COLOR_CYAN);
    console.print_colored("PID  STATE     NAME                    STACK", COLOR_WHITE);
    console.put_char('\n');

    // kshell (главный поток)
    console.print_colored("│ ", COLOR_CYAN);
    console.print_colored("  1  ", COLOR_GREEN);
    console.print_colored("Running   ", COLOR_GREEN);
    console.print("kshell                     main");
    console.put_char('\n');

    // idle (если есть)
    if task_count > 1 {
        console.print_colored("│ ", COLOR_CYAN);
        console.print_colored("  2  ", COLOR_YELLOW);
        console.print_colored("Sleeping  ", COLOR_YELLOW);
        console.print("idle                       hlt-loop");
        console.put_char('\n');
    }

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_CYAN);
    console.put_char('\n');
}

fn print_fs_info(console: &mut Console) {
    console.print_colored("┌─── Filesystem ───────────────────────────────────────────────┐\n", COLOR_WHITE);

    // Root filesystem
    let (heap_used, heap_total) = crate::memory::heap::stats();
    let used_kb = heap_used / 1024;
    let total_kb = heap_total / 1024;

    console.print_colored("│ ", COLOR_WHITE);
    console.print_colored("Mount  Size      Used      Avail     Type\n", COLOR_WHITE);

    console.print_colored("│ ", COLOR_WHITE);
    console.print("  /    ");
    console.print_num(total_kb);
    console.print("KB  ");
    console.print_num(used_kb);
    console.print("KB   ");
    console.print_num(total_kb - used_kb);
    console.print("KB    ramfs");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_WHITE);
    console.print("  /proc   0KB     0KB       0KB     procfs");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_WHITE);
    console.print("  /sys    0KB     0KB       0KB     sysfs");
    console.put_char('\n');

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_WHITE);
    console.put_char('\n');
}

/// Нарисовать прогресс-бар
fn draw_bar(console: &mut Console, percent: usize, width: usize) {
    let filled = (percent * width) / 100;
    let empty = width - filled;

    console.print_colored("[", COLOR_WHITE);

    if percent > 80 {
        for _ in 0..filled { console.print_colored("#", COLOR_RED); }
    } else if percent > 50 {
        for _ in 0..filled { console.print_colored("#", COLOR_YELLOW); }
    } else {
        for _ in 0..filled { console.print_colored("#", COLOR_GREEN); }
    }

    for _ in 0..empty { console.print_colored(".", COLOR_WHITE); }

    console.print_colored("]", COLOR_WHITE);
}
