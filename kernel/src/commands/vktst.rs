// vktst — Vibra Kernel Tests.
//
// Единая точка входа для системных проверок: CPU, память, таймер/RTC, ФС,
// блочные устройства, сеть, ядро и boot-лог. Заменяет удалённые `test`,
// `systest` и `heap`; глубокая диагностика подсистем остаётся в `diag`.
//
// Режимы:
//   vktst                 интерактивное меню
//   vktst list            список тестов
//   vktst run <id|all>    неинтерактивный прогон
//   vktst report          прогнать всё и сохранить отчёт в /var/log/vktst-*.log

use super::CmdResult;
use crate::framebuffer::{
    Console, COLOR_CYAN, COLOR_DARK_GRAY, COLOR_GREEN, COLOR_RED, COLOR_VIBRA_FG, COLOR_WHITE,
    COLOR_YELLOW,
};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

struct TestResult {
    name: &'static str,
    ok: bool,
    details: Vec<String>,
}

impl TestResult {
    fn new(name: &'static str, ok: bool) -> Self {
        Self {
            name,
            ok,
            details: Vec::new(),
        }
    }

    fn line(mut self, text: String) -> Self {
        self.details.push(text);
        self
    }
}

const TESTS: &[(&str, &str)] = &[
    ("cpu", "CPU topology, features, frequency"),
    ("mem", "PMM and heap invariants"),
    ("timer", "Monotonic clock and RTC"),
    ("fs", "RamFS/procfs read/write"),
    ("disk", "Block devices and filesystem types"),
    ("net", "Network interface state"),
    ("kernel", "Version, devices, drivers, modules"),
    ("log", "Boot log presence"),
];

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        return interactive(console);
    }

    match args[0] {
        "list" | "ls" => {
            print_list(console);
            CmdResult::Ok
        }
        "run" => {
            let target = args.get(1).copied().unwrap_or("all");
            run_selected(console, target);
            CmdResult::Ok
        }
        "report" => {
            let results = run_all(console);
            let path = save_report(&results);
            match path {
                Some(path) => {
                    console.print_colored("Report saved: ", COLOR_GREEN);
                    console.print(&path);
                    console.put_char('\n');
                }
                None => console.print_colored("Report could not be written\n", COLOR_RED),
            }
            CmdResult::Ok
        }
        "--help" | "-h" | "help" => {
            print_help(console);
            CmdResult::Ok
        }
        other => {
            console.print_colored("vktst: unknown mode '", COLOR_RED);
            console.print(other);
            console.print_colored("'\n", COLOR_RED);
            print_help(console);
            CmdResult::Ok
        }
    }
}

fn print_help(console: &mut Console) {
    console.print_colored("Usage:\n", COLOR_CYAN);
    console.print("  vktst                interactive menu\n");
    console.print("  vktst list           list tests\n");
    console.print("  vktst run <id|all>   run one test or all\n");
    console.print("  vktst report         run all and save /var/log/vktst-<uptime>.log\n");
    console.print("\nTests: ");
    for (id, _) in TESTS {
        console.print(id);
        console.print(" ");
    }
    console.put_char('\n');
}

fn print_list(console: &mut Console) {
    console.print_colored("Vibra Kernel Tests\n", COLOR_CYAN);
    for (index, (id, description)) in TESTS.iter().enumerate() {
        console.print("  ");
        console.print_num(index + 1);
        console.print("  ");
        console.print(id);
        let pad = 8usize.saturating_sub(id.len());
        for _ in 0..pad {
            console.put_char(' ');
        }
        console.print(description);
        console.put_char('\n');
    }
}

fn run_selected(console: &mut Console, target: &str) {
    if target == "all" {
        run_all(console);
        return;
    }

    let selected = target
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .or_else(|| TESTS.iter().position(|(id, _)| *id == target));

    let Some(index) = selected.filter(|i| *i < TESTS.len()) else {
        console.print_colored("vktst: no such test: ", COLOR_RED);
        console.print(target);
        console.put_char('\n');
        print_list(console);
        return;
    };

    let result = run_one(index);
    print_result(console, &result);
}

fn run_all(console: &mut Console) -> Vec<TestResult> {
    let mut results = Vec::new();
    for index in 0..TESTS.len() {
        if crate::poll_cancel() {
            console.print_colored("[cancelled]\n", COLOR_YELLOW);
            break;
        }
        let result = run_one(index);
        print_result(console, &result);
        results.push(result);
    }
    print_summary(console, &results);
    results
}

fn run_one(index: usize) -> TestResult {
    match TESTS[index].0 {
        "cpu" => test_cpu(),
        "mem" => test_memory(),
        "timer" => test_timer(),
        "fs" => test_fs(),
        "disk" => test_disk(),
        "net" => test_net(),
        "kernel" => test_kernel(),
        "log" => test_log(),
        _ => TestResult::new("unknown", false),
    }
}

fn test_cpu() -> TestResult {
    let info = crate::cpu_info::detect();
    let t = &info.topology;
    let mut ok = t.physical_cores >= 1
        && t.logical_cores >= t.physical_cores
        && t.threads_per_core >= 1
        && t.logical_cores % t.threads_per_core == 0;

    // Защита от старой ошибки: max addressable IDs округлялись до степени
    // двойки и выдавались за число ядер.
    if t.logical_cores > 1024 || t.physical_cores > 512 {
        ok = false;
    }

    let mut result = TestResult::new("cpu", ok);
    result = result.line(format!(
        "brand: {}",
        crate::cpu_info::brand_str(&info)
    ));
    result = result.line(format!(
        "vendor: {}, topology: {} ({} physical, {} logical, {} threads/core)",
        crate::cpu_info::vendor_str(&info),
        crate::cpu_info::topology_str(&info),
        t.physical_cores,
        t.logical_cores,
        t.threads_per_core
    ));
    result = result.line(format!(
        "apic id: {}, family/model/stepping: {}/{}/{}, freq: {}",
        t.apic_id,
        info.family,
        info.model,
        info.stepping,
        crate::cpu_info::freq_str(&info)
    ));

    let mut features = Vec::new();
    for (name, present) in [
        ("sse2", info.features.sse2),
        ("avx", info.features.avx),
        ("avx2", info.features.avx2),
        ("nx", info.features.nx),
        ("smep", info.features.smep),
        ("smap", info.features.smap),
        ("tsc", info.features.tsc),
        ("apic", info.features.apic),
    ] {
        if present {
            features.push(name);
        }
    }
    result.line(format!("features: {}", features.join(" ")))
}

fn test_memory() -> TestResult {
    let mem = crate::memory::meminfo();
    let mut ok = mem.total_bytes > 0
        && mem.used_bytes <= mem.total_bytes
        && mem.heap_used <= mem.heap_total;

    // Небольшой стресс: полный (10000 итераций) остаётся в `diag heap-test`.
    let (iters, stress_ok, drift) = crate::memory::heap::stress(128);
    if !stress_ok {
        ok = false;
    }

    TestResult::new("mem", ok)
        .line(format!(
            "ram: {} used, {} total, {} free ({}%)",
            crate::memory::format_bytes(mem.used_bytes),
            crate::memory::format_bytes(mem.total_bytes),
            crate::memory::format_bytes(mem.free_bytes),
            mem.used_percent()
        ))
        .line(format!(
            "heap: {} / {}",
            crate::memory::format_bytes(mem.heap_used),
            crate::memory::format_bytes(mem.heap_total)
        ))
        .line(format!(
            "heap stress: {} iters, {}, drift {} bytes",
            iters,
            if stress_ok { "OK" } else { "FAILED" },
            drift
        ))
}

fn test_timer() -> TestResult {
    let start = crate::timer::uptime_ns();
    // Короткое окно: проверяем, что монотонные часы идут.
    let mut guard = 0u32;
    while crate::timer::uptime_ns().saturating_sub(start) < 5_000_000 && guard < 10_000_000 {
        guard += 1;
        core::hint::spin_loop();
    }
    let elapsed = crate::timer::uptime_ns().saturating_sub(start);
    let source = crate::timer::current_source();
    let info = crate::timer::clock_info();
    let rtc = crate::timer::wall_clock();

    let mut ok = elapsed > 0 && source != crate::timer::ClockSource::None;

    let mut result = TestResult::new("timer", ok)
        .line(format!("source: {}", source.name()))
        .line(format!(
            "uptime: {}.{:03} s, measured {} ns",
            crate::timer::uptime_ms() / 1000,
            crate::timer::uptime_ms() % 1000,
            elapsed
        ))
        .line(format!(
            "irq ticks: {}, healthy: {}",
            info.irq_ticks, info.irq_healthy
        ));

    match rtc {
        Some(dt) => result = result.line(format!(
            "rtc: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
        )),
        None => {
            ok = false;
            result = result.line(String::from("rtc: unavailable"));
        }
    }
    result.ok = ok;
    result
}

fn test_fs() -> TestResult {
    let root_entries = crate::fs::list_dir("/");
    let has_proc = root_entries.iter().any(|entry| entry.name == "proc");
    let has_var = root_entries.iter().any(|entry| entry.name == "var");

    let proc_ok = crate::fs::read_file("/proc/uptime").is_ok();

    let probe = "/tmp/vktst-probe.txt";
    let write_ok = crate::fs::write_file(probe, b"vktst\n").is_ok();
    let read_ok = crate::fs::read_file(probe)
        .map(|data| data == b"vktst\n")
        .unwrap_or(false);
    let _ = crate::fs::remove_entry(probe);

    let mounts = crate::fs::list_mounts();
    let ok = has_proc && has_var && proc_ok && write_ok && read_ok;

    let mut result = TestResult::new("fs", ok)
        .line(format!("root entries: {}", root_entries.len()))
        .line(format!(
            "procfs: {}, ramfs write: {}, read-back: {}",
            if proc_ok { "OK" } else { "FAIL" },
            if write_ok { "OK" } else { "FAIL" },
            if read_ok { "OK" } else { "FAIL" }
        ));
    result = result.line(format!("mounts: {}", mounts.len()));
    for mount in mounts.iter().take(8) {
        result = result.line(format!("  {}", mount));
    }
    result
}

fn test_disk() -> TestResult {
    let mgr = crate::fs::get_disk_manager().lock();
    let count = mgr.count();
    let mut result = TestResult::new("disk", count > 0);
    result = result.line(format!("block devices: {}", count));
    for (index, entry) in mgr.entries().iter().enumerate() {
        if index >= 12 {
            result = result.line(String::from("  ..."));
            break;
        }
        result = result.line(format!(
            "  {}: {} {} MiB fs={}",
            entry.name,
            entry.kind,
            entry.size_mb(),
            entry.fs_type
        ));
    }
    let (read, write) = crate::fs::disk::io_stats();
    result.line(format!(
        "io: read {}, write {}",
        crate::memory::format_bytes(read),
        crate::memory::format_bytes(write)
    ))
}

fn test_net() -> TestResult {
    let initialized = crate::net::is_initialized();
    let mac = crate::net::get_local_mac();
    let ip = crate::net::get_local_ip();
    let (rx_packets, rx_bytes, tx_packets, tx_bytes) = crate::net::stats();

    TestResult::new("net", initialized)
        .line(format!(
            "mac: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ))
        .line(format!("ip: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
        .line(format!(
            "rx: {} / {} pkts, tx: {} / {} pkts",
            crate::memory::format_bytes(rx_bytes),
            rx_packets,
            crate::memory::format_bytes(tx_bytes),
            tx_packets
        ))
}

fn test_kernel() -> TestResult {
    let devices = crate::kernel::registry::device_count();
    let drivers = crate::kernel::registry::driver_count();
    let modules = crate::kernel::registry::module_count();

    TestResult::new("kernel", modules > 0)
        .line(format!(
            "version: {} \"{}\"",
            crate::version::KERNEL_VERSION,
            crate::version::KERNEL_CODENAME
        ))
        .line(format!(
            "devices: {}, drivers: {}, modules: {}",
            devices, drivers, modules
        ))
}

fn test_log() -> TestResult {
    match crate::fs::read_file("/var/log/boot.log") {
        Ok(data) => TestResult::new("log", !data.is_empty())
            .line(format!("boot.log: {} bytes", data.len()))
            .line(format!("sink attached: {}", crate::boot_log::usb::is_attached())),
        Err(error) => TestResult::new("log", false)
            .line(format!("boot.log: {}", error)),
    }
}

fn print_result(console: &mut Console, result: &TestResult) {
    console.print_colored(
        if result.ok { "[ OK ] " } else { "[FAIL] " },
        if result.ok { COLOR_GREEN } else { COLOR_RED },
    );
    console.print(result.name);
    console.put_char('\n');
    for detail in &result.details {
        console.print("       ");
        console.print(detail);
        console.put_char('\n');
    }
}

fn print_summary(console: &mut Console, results: &[TestResult]) {
    let passed = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - passed;
    console.put_char('\n');
    console.print_colored("Summary: ", COLOR_WHITE);
    console.print_num(passed);
    console.print(" passed, ");
    if failed > 0 {
        console.print_colored(&format!("{} failed", failed), COLOR_RED);
    } else {
        console.print_colored("0 failed", COLOR_GREEN);
    }
    console.put_char('\n');
}

fn save_report(results: &[TestResult]) -> Option<String> {
    let mut body = String::new();
    body.push_str("Vibra Kernel Tests report\n");
    body.push_str(&format!(
        "version: {} \"{}\"\n",
        crate::version::KERNEL_VERSION,
        crate::version::KERNEL_CODENAME
    ));
    body.push_str(&format!("uptime_ms: {}\n\n", crate::timer::uptime_ms()));

    for result in results {
        body.push_str(&format!(
            "[{}] {}\n",
            if result.ok { " OK " } else { "FAIL" },
            result.name
        ));
        for detail in &result.details {
            body.push_str("       ");
            body.push_str(detail);
            body.push('\n');
        }
    }

    let passed = results.iter().filter(|r| r.ok).count();
    body.push_str(&format!(
        "\nsummary: {} passed, {} failed\n",
        passed,
        results.len() - passed
    ));

    let path = format!("/var/log/vktst-{}.log", crate::timer::uptime_ms() / 1000);
    crate::fs::write_file(&path, body.as_bytes()).ok()?;
    Some(path)
}

fn interactive(console: &mut Console) -> CmdResult {
    loop {
        console.clear();
        console.print_colored("Vibra Kernel Tests (vktst)\n", COLOR_CYAN);
        console.print_colored("----------------------------\n", COLOR_DARK_GRAY);
        for (index, (id, description)) in TESTS.iter().enumerate() {
            console.print("  ");
            console.print_num(index + 1);
            console.print("  ");
            console.print(id);
            let pad = 8usize.saturating_sub(id.len());
            for _ in 0..pad {
                console.put_char(' ');
            }
            console.print(description);
            console.put_char('\n');
        }
        console.put_char('\n');
        console.print_colored("  a  run all tests\n", COLOR_WHITE);
        console.print_colored("  l  run all and save report\n", COLOR_WHITE);
        console.print_colored("  q  quit\n", COLOR_WHITE);
        console.print_colored("\nChoice: ", COLOR_VIBRA_FG);

        let choice = wait_key();
        console.put_char('\n');

        match choice {
            Some('q') | Some('\x03') | Some('\x1A') | None => break,
            Some('a') => {
                let _ = run_all(console);
                pause(console);
            }
            Some('l') => {
                let results = run_all(console);
                match save_report(&results) {
                    Some(path) => {
                        console.print_colored("Report saved: ", COLOR_GREEN);
                        console.print(&path);
                        console.put_char('\n');
                    }
                    None => console.print_colored("Report could not be written\n", COLOR_RED),
                }
                pause(console);
            }
            Some(ch) if ch.is_ascii_digit() => {
                let index = ch as usize - '0' as usize;
                if index >= 1 && index <= TESTS.len() {
                    let result = run_one(index - 1);
                    print_result(console, &result);
                }
                pause(console);
            }
            _ => {}
        }
    }

    crate::reset_cancel();
    CmdResult::Ok
}

fn wait_key() -> Option<char> {
    loop {
        if let Some(key) = crate::serial::poll_key().or_else(crate::keyboard::poll_key) {
            return match key {
                crate::keyboard::Key::Char(ch) => Some(ch),
                crate::keyboard::Key::Enter => Some('\n'),
                _ => continue,
            };
        }
        crate::timer::idle_wait();
    }
}

fn pause(console: &mut Console) {
    console.print_colored("\nPress any key...", COLOR_DARK_GRAY);
    let _ = wait_key();
}
