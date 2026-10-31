// top — интерактивный системный монитор в духе htop.
//
// Показывает:
//   * загрузку CPU (агрегат; SMP ещё не включён, поэтому per-core был бы
//     фикцией — остальные логические CPU помечены как неактивные);
//   * память в человекочитаемом виде (GiB/MiB) и процент;
//   * суммарный дисковый I/O и сетевой трафик;
//   * процессы с PID, состоянием, CPU% и временем.
//
// `top -b` / `top --batch` печатает один кадр и выходит — это режим для CI.

use super::CmdResult;
use crate::framebuffer::{
    Console, COLOR_CYAN, COLOR_DARK_GRAY, COLOR_GREEN, COLOR_RED, COLOR_VIBRA_FG, COLOR_WHITE,
    COLOR_YELLOW,
};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Сколько миллисекунд измеряем загрузку перед кадром.
const SAMPLE_MS: u64 = 900;
/// В batch-режиме хватает короткого окна.
const BATCH_SAMPLE_MS: u64 = 350;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let batch = args.iter().any(|a| *a == "-b" || *a == "--batch");

    if batch {
        let before = snapshot_tasks();
        let (load, _) = sample_load(BATCH_SAMPLE_MS, false);
        let rows = process_rows(&before, BATCH_SAMPLE_MS * 1_000_000);
        draw(console, load, &rows, false);
        return CmdResult::Ok;
    }

    loop {
        let before = snapshot_tasks();
        let (load, quit) = sample_load(SAMPLE_MS, true);
        if quit {
            break;
        }
        let rows = process_rows(&before, SAMPLE_MS * 1_000_000);

        console.clear();
        draw(console, load, &rows, true);

        if crate::is_cancelled() {
            break;
        }
    }

    crate::reset_cancel();
    CmdResult::Ok
}

/// Измерить загрузку CPU за окно `duration_ms`.
///
/// Считаем не «context switches per second» (старый способ давал 24% на
/// простаивающем shell), а долю времени, которую CPU не провёл в idle:
/// `timer::idle_wait` аккумулирует время в `hlt`/`spin_loop`.
fn sample_load(duration_ms: u64, poll_keys: bool) -> (u32, bool) {
    let wall_start = crate::timer::uptime_ns();
    let idle_start = crate::timer::idle_ns();
    let deadline = wall_start.saturating_add(duration_ms.saturating_mul(1_000_000));
    let mut quit = false;

    loop {
        if poll_keys {
            if let Some(key) = crate::serial::poll_key().or_else(crate::keyboard::poll_key) {
                match key {
                    crate::keyboard::Key::Char('q')
                    | crate::keyboard::Key::Char('Q')
                    | crate::keyboard::Key::Char('\x03')
                    | crate::keyboard::Key::Char('\x1A') => {
                        quit = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        let now = crate::timer::uptime_ns();
        if now >= deadline {
            break;
        }
        crate::timer::idle_wait();
    }

    let wall = crate::timer::uptime_ns().saturating_sub(wall_start);
    let idle = crate::timer::idle_ns().saturating_sub(idle_start);
    let load = if wall == 0 {
        0
    } else {
        100u64.saturating_sub(idle.saturating_mul(100) / wall)
    };
    (load.min(100) as u32, quit)
}

fn snapshot_tasks() -> Vec<(u32, u64)> {
    crate::task::list_tasks_detail()
        .into_iter()
        .map(|(pid, _, _, cpu_ns, _)| (pid, cpu_ns))
        .collect()
}

struct Row {
    pid: u32,
    state: &'static str,
    cpu_pct: u32,
    cpu_ns: u64,
    name: String,
}

fn process_rows(previous: &[(u32, u64)], elapsed_ns: u64) -> Vec<Row> {
    let tasks = crate::task::list_tasks_detail();
    let uptime = crate::timer::uptime_ns().max(1);
    let elapsed = elapsed_ns.max(1);
    let mut rows = Vec::new();

    for (pid, name, state, cpu_ns, _) in tasks {
        // Для уже виденных PID считаем дельту за окно измерения. Для новых —
        // среднее за всё время жизни задачи.
        let cpu_pct = if let Some((_, prev_cpu)) = previous.iter().find(|(p, _)| *p == pid) {
            let delta = cpu_ns.saturating_sub(*prev_cpu);
            (delta.saturating_mul(100) / elapsed).min(100) as u32
        } else {
            (cpu_ns.saturating_mul(100) / uptime).min(100) as u32
        };

        rows.push(Row {
            pid,
            state,
            cpu_pct,
            cpu_ns,
            name,
        });
    }

    // Самый нагруженный процесс сверху; при равенстве — по PID.
    rows.sort_by(|a, b| b.cpu_pct.cmp(&a.cpu_pct).then(a.pid.cmp(&b.pid)));

    rows
}

fn draw(console: &mut Console, load: u32, rows: &[Row], interactive: bool) {
    let mem = crate::memory::meminfo();
    let cpu = crate::cpu_info::detect();
    let uptime = crate::timer::uptime_ms();
    let (tasks, switches, _) = crate::task::stats();
    let (disk_read, disk_write) = crate::fs::disk::io_stats();
    let (rx_packets, rx_bytes, tx_packets, tx_bytes) = crate::net::stats();

    console.print_colored("Vibra top", COLOR_CYAN);
    console.print("  up ");
    console.print(&format_uptime(uptime));
    console.print("   load: ");
    print_pct(console, load);
    console.print("   ticks: ");
    console.print_num(tasks as usize);
    console.print("   switches: ");
    console.print_num(switches as usize);
    console.put_char('\n');

    // CPU
    console.print_colored("CPU  ", COLOR_YELLOW);
    draw_bar(console, load as usize, 40);
    console.print(" ");
    print_pct(console, load);
    let topology = crate::cpu_info::topology_str(&cpu);
    console.print("  ");
    console.print(&topology);
    console.print(" detected, ");
    console.print_num(1);
    console.print(" active (SMP not enabled)");
    console.put_char('\n');

    // Memory
    console.print_colored("Mem  ", COLOR_GREEN);
    draw_bar(console, mem.used_percent() as usize, 40);
    console.print(" ");
    print_pct(console, mem.used_percent());
    console.print("  ");
    console.print(&crate::memory::format_pair(mem.used_bytes, mem.total_bytes));
    console.print("  free ");
    console.print(&crate::memory::format_bytes(mem.free_bytes));
    console.put_char('\n');

    console.print_colored("Heap ", COLOR_GREEN);
    let heap_pct = if mem.heap_total > 0 {
        ((mem.heap_used * 100) / mem.heap_total) as u32
    } else {
        0
    };
    draw_bar(console, heap_pct as usize, 40);
    console.print(" ");
    print_pct(console, heap_pct);
    console.print("  ");
    console.print(&crate::memory::format_pair(mem.heap_used, mem.heap_total));
    console.put_char('\n');

    // Disk / net
    console.print_colored("Disk ", COLOR_YELLOW);
    console.print("read ");
    console.print(&crate::memory::format_bytes(disk_read));
    console.print("   write ");
    console.print(&crate::memory::format_bytes(disk_write));
    console.put_char('\n');

    console.print_colored("Net  ", COLOR_YELLOW);
    console.print("RX ");
    console.print(&crate::memory::format_bytes(rx_bytes));
    console.print(" / ");
    console.print_num(rx_packets as usize);
    console.print(" pkts   TX ");
    console.print(&crate::memory::format_bytes(tx_bytes));
    console.print(" / ");
    console.print_num(tx_packets as usize);
    console.print(" pkts");
    console.put_char('\n');

    console.put_char('\n');

    // Process table
    console.print_colored(
        "  PID  STATE    CPU%    TIME      NAME\n",
        COLOR_WHITE,
    );
    console.print_colored(
        "─────  ───────  ─────  ────────  ────────────────────────\n",
        COLOR_DARK_GRAY,
    );

    let max_rows = console.rows().saturating_sub(11).max(1);
    for row in rows.iter().take(max_rows) {
        console.print("  ");
        console.print_num(row.pid as usize);
        let pad = 5usize.saturating_sub(row.pid.to_string().len());
        for _ in 0..pad {
            console.put_char(' ');
        }
        console.print("  ");

        match row.state {
            "Running" => console.print_colored("Running", COLOR_GREEN),
            "Ready" => console.print_colored("Ready  ", COLOR_YELLOW),
            "Sleep" => console.print_colored("Sleep  ", COLOR_CYAN),
            "Zombie" => console.print_colored("Zombie ", COLOR_RED),
            _ => console.print_colored("Block  ", COLOR_WHITE),
        }

        console.print("  ");
        let pct = format!("{:>3}", row.cpu_pct);
        if row.cpu_pct > 50 {
            console.print_colored(&pct, COLOR_RED);
        } else if row.cpu_pct > 10 {
            console.print_colored(&pct, COLOR_YELLOW);
        } else {
            console.print(&pct);
        }
        console.print("    ");
        console.print(&format_cpu_time(row.cpu_ns));
        console.print("  ");
        console.print(&row.name);
        console.put_char('\n');
    }

    if interactive {
        console.print_colored(
            "\n q / Ctrl+C — quit, r — refresh\n",
            COLOR_VIBRA_FG,
        );
    }
}

fn print_pct(console: &mut Console, pct: u32) {
    console.print_num(pct as usize);
    console.print("%");
}

fn draw_bar(console: &mut Console, percent: usize, width: usize) {
    let percent = percent.min(100);
    let filled = (percent * width) / 100;
    let empty = width.saturating_sub(filled);

    console.print_colored("[", COLOR_WHITE);
    let color = if percent > 80 {
        COLOR_RED
    } else if percent > 50 {
        COLOR_YELLOW
    } else {
        COLOR_GREEN
    };
    for _ in 0..filled {
        console.print_colored("#", color);
    }
    for _ in 0..empty {
        console.print_colored(".", COLOR_WHITE);
    }
    console.print_colored("]", COLOR_WHITE);
}

fn format_uptime(ms: u64) -> String {
    let secs = ms / 1000;
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{}d {:02}:{:02}:{:02}", days, hours, minutes, seconds)
}

fn format_cpu_time(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    format!("{:02}:{:02}", secs / 60, secs % 60)
}
