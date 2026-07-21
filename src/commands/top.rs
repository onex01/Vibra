use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_YELLOW, COLOR_CYAN, COLOR_RED, COLOR_WHITE};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let _show_once = args.iter().any(|a| *a == "-b" || *a == "--batch");
    print_stats(console);
    CmdResult::Ok
}

fn print_stats(console: &mut Console) {
    print_cpu_info(console);
    print_memory_info(console);
    print_processes(console);
}

fn print_cpu_info(console: &mut Console) {
    console.print_colored("┌─── CPU ──────────────────────────────────────────────────────┐\n", COLOR_YELLOW);

    let info = crate::cpu_info::detect();

    // Имя процессора
    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("CPU:    ", COLOR_WHITE);
    console.print(crate::cpu_info::brand_str(&info));
    console.put_char('\n');

    // Ядра
    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Cores:  ", COLOR_WHITE);
    console.print_num(info.cores as usize);
    console.print(" logical");
    console.put_char('\n');

    // Частота
    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Freq:   ", COLOR_WHITE);
    console.print(crate::cpu_info::freq_str(&info).as_str());
    console.put_char('\n');

    // Uptime
    let (sched_ticks, ctx_sw, _) = crate::task::stats();
    let uptime_secs = sched_ticks / 100;
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
    console.put_char('\n');

    // Scheduler stats
    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Ticks:  ", COLOR_WHITE);
    console.print_num(sched_ticks as usize);
    console.print("  Switches: ");
    console.print_num(ctx_sw as usize);
    console.put_char('\n');

    // CPU load: idle ticks / total ticks * 100 = idle%
    // Load = 100% - idle%
    let (busy, idle) = crate::task::cpu_load();
    let total = busy + idle;
    let idle_pct = if total > 0 { (idle * 100 / total) as usize } else { 0 };
    let load_pct = 100 - idle_pct;

    console.print_colored("│ ", COLOR_YELLOW);
    console.print_colored("Load:   ", COLOR_WHITE);
    draw_bar(console, load_pct, 30);
    console.print(" ~");
    console.print_num(load_pct);
    console.print("%");
    console.put_char('\n');

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_YELLOW);
    console.put_char('\n');
}

fn print_memory_info(console: &mut Console) {
    console.print_colored("┌─── Memory ───────────────────────────────────────────────────┐\n", COLOR_GREEN);

    let (used_frames, total_frames) = crate::memory::pmm::stats();
    let (heap_used, heap_total) = crate::memory::heap::stats();

    // ОС использует: PMM (physical frames) + heap
    // PMM: сколько фреймов занято ядром, boot, page tables, etc.
    let total_mb = (total_frames * 4096) / (1024 * 1024);
    let used_mb = (used_frames * 4096) / (1024 * 1024);
    let free_mb = total_mb - used_mb;

    let used_kb = (used_frames * 4096) / 1024;
    let total_kb = (total_frames * 4096) / 1024;
    let free_kb = total_kb - used_kb;

    let mem_pct = if total_kb > 0 { (used_kb * 100) / total_kb } else { 0 };

    // Physical RAM
    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("RAM:    ", COLOR_WHITE);
    console.print_num(used_mb);
    console.print(" / ");
    console.print_num(total_mb);
    console.print(" MB (");
    console.print_num(used_kb);
    console.print(" / ");
    console.print_num(total_kb);
    console.print(" KB)");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Used:   ", COLOR_WHITE);
    draw_bar(console, mem_pct, 30);
    console.print(" ");
    console.print_num(mem_pct);
    console.print("%");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Free:   ", COLOR_WHITE);
    console.print_num(free_mb);
    console.print(" MB (");
    console.print_num(free_kb);
    console.print(" KB)");
    console.put_char('\n');

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Frames: ", COLOR_WHITE);
    console.print_num(used_frames);
    console.print(" used / ");
    console.print_num(total_frames);
    console.print(" total");
    console.put_char('\n');

    // Heap
    let heap_used_kb = heap_used / 1024;
    let heap_total_kb = heap_total / 1024;
    let heap_pct = if heap_total > 0 { (heap_used * 100) / heap_total } else { 0 };

    console.print_colored("│ ", COLOR_GREEN);
    console.print_colored("Heap:   ", COLOR_WHITE);
    console.print_num(heap_used_kb);
    console.print(" / ");
    console.print_num(heap_total_kb);
    console.print(" KB (");
    draw_bar(console, heap_pct, 15);
    console.print(" ");
    console.print_num(heap_pct);
    console.print("%)");
    console.put_char('\n');

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_GREEN);
    console.put_char('\n');
}

fn print_processes(console: &mut Console) {
    console.print_colored("┌─── Processes ────────────────────────────────────────────────┐\n", COLOR_CYAN);

    let tasks = crate::task::list_tasks();

    console.print_colored("│ ", COLOR_CYAN);
    console.print_colored("PID  STATE     NAME\n", COLOR_WHITE);

    for (id, name, state) in &tasks {
        console.print_colored("│ ", COLOR_CYAN);
        if *id < 10 { console.print(" "); }
        console.print("  ");
        console.print_num(*id as usize);
        console.print("  ");

        match *state {
            "Running" => console.print_colored(state, COLOR_GREEN),
            "Ready" => console.print_colored(state, COLOR_YELLOW),
            _ => console.print_colored(state, COLOR_WHITE),
        }

        let padding = 8 - state.len();
        for _ in 0..padding { console.put_char(' '); }

        console.print(name);
        console.put_char('\n');
    }

    console.print_colored("└──────────────────────────────────────────────────────────────┘\n", COLOR_CYAN);
    console.put_char('\n');
}

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
