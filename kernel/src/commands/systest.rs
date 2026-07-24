// systest — интерактивное меню системного тестирования.
// Пункты: CPU, память, PCI, диски, сеть, ping, USB, APIC, статистика, выход.

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_WHITE, COLOR_RED};
use crate::keyboard::{self, Key};

/// Считать одну клавишу (с ожиданием). Возвращает Some(Key) или None если отмена.
fn read_choice(console: &mut Console) -> Option<u8> {
    loop {
        if crate::is_cancelled() {
            crate::reset_cancel();
            return None;
        }
        crate::task::yield_now();
        let key = crate::serial::poll_key().or_else(keyboard::poll_key);
        if let Some(k) = key {
            match k {
                Key::Char(ch) => {
                    let b = ch as u8;
                    if b >= b'0' && b <= b'9' {
                        return Some(b - b'0');
                    }
                    return None;
                }
                _ => {}
            }
        }
    }
}

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    loop {
        if crate::is_cancelled() {
            crate::reset_cancel();
            return CmdResult::Ok;
        }

        console.print_colored("\n╔════════════════════════════════════╗\n", COLOR_CYAN);
        console.print_colored("║      Vibra OS — System Test       ║\n", COLOR_CYAN);
        console.print_colored("╠════════════════════════════════════╣\n", COLOR_CYAN);
        console.print_colored("║  1. CPU info                      ║\n", COLOR_WHITE);
        console.print_colored("║  2. Memory map                    ║\n", COLOR_WHITE);
        console.print_colored("║  3. PCI devices                   ║\n", COLOR_WHITE);
        console.print_colored("║  4. Disk info                     ║\n", COLOR_WHITE);
        console.print_colored("║  5. Network info                  ║\n", COLOR_WHITE);
        console.print_colored("║  6. Ping test                     ║\n", COLOR_WHITE);
        console.print_colored("║  7. USB devices                   ║\n", COLOR_WHITE);
        console.print_colored("║  8. APIC status                   ║\n", COLOR_WHITE);
        console.print_colored("║  9. Kernel stats                  ║\n", COLOR_WHITE);
        console.print_colored("║  0. Quit                          ║\n", COLOR_WHITE);
        console.print_colored("╚════════════════════════════════════╝\n", COLOR_CYAN);
        console.print_colored("Выберите пункт: ", COLOR_GREEN);

        let choice = match read_choice(console) {
            Some(c) => c,
            None => return CmdResult::Ok,
        };

        match choice {
            1 => show_cpu_info(console),
            2 => show_memory_map(console),
            3 => show_pci_devices(console),
            4 => show_disk_info(console),
            5 => show_network_info(console),
            6 => run_ping_test(console),
            7 => show_usb_devices(console),
            8 => show_apic_status(console),
            9 => show_kernel_stats(console),
            0 => {
                console.print_colored("Выход из systest.\n", COLOR_YELLOW);
                return CmdResult::Ok;
            }
            _ => {
                console.print_colored("Неизвестный пункт: ", COLOR_RED);
                console.print_num(choice as usize);
                console.print("\n");
            }
        }
    }
}

/// Информация о процессоре (CPUID)
fn show_cpu_info(console: &mut Console) {
    console.print_colored("\n--- CPU Info ---\n", COLOR_YELLOW);
    let info = crate::cpu_info::detect();
    let brand = crate::cpu_info::brand_str(&info);
    console.print("  CPU:    ");
    console.print(brand);
    console.print("\n");
    console.print("  Vendor: ");
    console.print(crate::cpu_info::vendor_str(&info));
    console.print("\n");
    console.print("  Cores:  ");
    console.print_num(info.cores as usize);
    console.print(" logical\n");
    console.print("  Freq:   ");
    console.print(crate::cpu_info::freq_str(&info).as_str());
    console.print("\n");
    console.print("  Family: ");
    console.print_num(info.family as usize);
    console.print("  Model: ");
    console.print_num(info.model as usize);
    console.print("  Step: ");
    console.print_num(info.stepping as usize);
    console.print("\n");
    console.print("  Max CPUID leaf:      ");
    console.print_num(info.max_leaf as usize);
    console.print("\n");
    console.print("  Max CPUID ext leaf:  ");
    console.print_num(info.max_ext_leaf as usize);
    console.print("\n");

    // Feature flags
    let f = &info.features;
    console.print_colored("  Features: ", COLOR_GREEN);
    if f.sse    { console.print("SSE "); }
    if f.sse2   { console.print("SSE2 "); }
    if f.sse3   { console.print("SSE3 "); }
    if f.ssse3  { console.print("SSSE3 "); }
    if f.sse4_1 { console.print("SSE4.1 "); }
    if f.sse4_2 { console.print("SSE4.2 "); }
    if f.avx    { console.print("AVX "); }
    if f.avx2   { console.print("AVX2 "); }
    if f.mmx    { console.print("MMX "); }
    if f.fpu    { console.print("FPU "); }
    if f.nx     { console.print("NX "); }
    if f.apic   { console.print("APIC "); }
    if f.tsc    { console.print("TSC "); }
    if f.smep   { console.print("SMEP "); }
    if f.smap   { console.print("SMAP "); }
    if f.lm     { console.print("LM(64) "); }
    console.print("\n");
}

/// Карта памяти (из Limine MemmapRequest)
fn show_memory_map(console: &mut Console) {
    console.print_colored("\n--- Memory Map ---\n", COLOR_YELLOW);
    match crate::MEMORY_MAP_REQUEST.response() {
        Some(mm) => {
            let entries = mm.entries();
            console.print("  Entries: ");
            console.print_num(entries.len());
            console.print("\n\n");

            let mut total_usable: u64 = 0;
            for (i, entry) in entries.iter().enumerate() {
                let base = entry.base;
                let len = entry.length;
                let end = base + len;
                let size_mb = len / (1024 * 1024);

                let type_str = match entry.type_ {
                    limine::memmap::MEMMAP_USABLE                => "USABLE",
                    limine::memmap::MEMMAP_RESERVED              => "RESERVED",
                    limine::memmap::MEMMAP_ACPI_RECLAIMABLE      => "ACPI_RECLAIM",
                    limine::memmap::MEMMAP_ACPI_NVS              => "ACPI_NVS",
                    limine::memmap::MEMMAP_BAD_MEMORY            => "BAD",
                    limine::memmap::MEMMAP_BOOTLOADER_RECLAIMABLE => "BOOT_RECLAIM",
                    limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES => "KERNEL",
                    limine::memmap::MEMMAP_FRAMEBUFFER           => "FRAMEBUFFER",
                    _ => "UNKNOWN",
                };

                console.print("  [");
                if i < 10 { console.print(" "); }
                console.print_num(i);
                console.print("] 0x");
                print_hex64(console, base);
                console.print(" - 0x");
                print_hex64(console, end);
                console.print("  ");
                console.print_num(size_mb as usize);
                console.print("MB  ");
                console.print(type_str);
                console.print("\n");

                if entry.type_ == limine::memmap::MEMMAP_USABLE {
                    total_usable += len;
                }
            }

            let usable_mb = total_usable / (1024 * 1024);
            console.print("\n  Total usable: ");
            console.print_num(usable_mb as usize);
            console.print(" MB\n");
        }
        None => {
            console.print_colored("  Memory map недоступен\n", COLOR_RED);
        }
    }
}

/// Вспомогательная функция для печати 64-bit hex
fn print_hex64(console: &mut Console, val: u64) {
    let hex_chars: [u8; 16] = *b"0123456789ABCDEF";
    let mut buf = [0u8; 16];
    for i in 0..16 {
        buf[15 - i] = hex_chars[((val >> (i * 4)) & 0xF) as usize];
    }
    // Пропускаем ведущие нули
    let mut start = 0;
    while start < 15 && buf[start] == b'0' { start += 1; }
    if let Ok(s) = core::str::from_utf8(&buf[start..]) {
        console.print(s);
    }
}

/// Список PCI устройств
fn show_pci_devices(console: &mut Console) {
    console.print_colored("\n--- PCI Devices ---\n", COLOR_YELLOW);
    let count = crate::drivers::pci::device_count();
    console.print("  Found: ");
    console.print_num(count);
    console.print("\n\n");

    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                unsafe {
                    let vendor = crate::drivers::pci::pci_read_u16_config(bus, dev, func, 0x00);
                    if vendor == 0xFFFF { if func == 0 { break; } continue; }
                    let device = crate::drivers::pci::pci_read_u16_config(bus, dev, func, 0x02);
                    let class = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0B);
                    let sub = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0A);
                    let hdr = crate::drivers::pci::pci_read_u8_config(bus, dev, func, 0x0E);

                    let name = match (class, sub) {
                        (0x01, 0x06) => "AHCI/SATA",
                        (0x01, 0x08) => "NVMe",
                        (0x01, _) => "Storage",
                        (0x02, 0x00) => "Ethernet",
                        (0x02, _) => "Network",
                        (0x03, 0x00) => "VGA",
                        (0x06, 0x00) => "Host Bridge",
                        (0x06, 0x01) => "ISA Bridge",
                        (0x06, 0x04) => "PCI Bridge",
                        (0x0C, 0x03) => "USB",
                        _ => "Other",
                    };

                    console.print("  ");
                    print_hex2(console, bus);
                    console.print(":");
                    print_hex2(console, dev);
                    console.print(".");
                    console.print_num(func as usize);
                    console.print("  ");
                    print_hex4(console, vendor);
                    console.print(" ");
                    print_hex4(console, device);
                    console.print("  ");
                    console.print(name);
                    console.print("\n");

                    if func == 0 && hdr & 0x80 == 0 { break; }
                }
            }
        }
    }
}

fn print_hex2(console: &mut Console, val: u8) {
    let hex: [u8; 16] = *b"0123456789ABCDEF";
    let hi = hex[((val >> 4) & 0xF) as usize];
    let lo = hex[(val & 0xF) as usize];
    console.put_char(hi as char);
    console.put_char(lo as char);
}

fn print_hex4(console: &mut Console, val: u16) {
    print_hex2(console, (val >> 8) as u8);
    print_hex2(console, val as u8);
}

/// Информация о дисках
fn show_disk_info(console: &mut Console) {
    console.print_colored("\n--- Disk Info ---\n", COLOR_YELLOW);

    let (heap_used, heap_total) = crate::memory::heap::stats();
    let (used_frames, total_frames) = crate::memory::pmm::stats();

    console.print("  RAM disk (ramfs):\n");
    console.print("    Heap:  ");
    console.print_num(heap_used / 1024);
    console.print(" / ");
    console.print_num(heap_total / 1024);
    console.print(" KB\n");

    let total_mb = (total_frames * 4096) / (1024 * 1024);
    let used_mb = (used_frames * 4096) / (1024 * 1024);
    console.print("    RAM:   ");
    console.print_num(used_mb);
    console.print(" / ");
    console.print_num(total_mb);
    console.print(" MB\n");

    // Список смонтированных
    console.print("  Filesystems:\n");
    console.print("    ramfs   /\n");
    console.print("    procfs  /proc\n");
    console.print("    sysfs   /sys\n");
    console.print("    devtmpfs /dev\n");
}

/// Информация о сети
fn show_network_info(console: &mut Console) {
    console.print_colored("\n--- Network Info ---\n", COLOR_YELLOW);

    if !crate::net::is_initialized() {
        console.print_colored("  Сеть не инициализирована\n", COLOR_RED);
        return;
    }

    let mac = crate::net::get_local_mac();
    let ip = crate::net::get_local_ip();
    let gw = crate::net::get_gateway();
    let mask = crate::net::get_subnet_mask();

    console.print("  Interface: eth0\n");
    console.print("  MAC: ");
    print_hex2(console, mac[0]);
    console.print(":");
    print_hex2(console, mac[1]);
    console.print(":");
    print_hex2(console, mac[2]);
    console.print(":");
    print_hex2(console, mac[3]);
    console.print(":");
    print_hex2(console, mac[4]);
    console.print(":");
    print_hex2(console, mac[5]);
    console.print("\n");
    console.print("  IP:    ");
    console.print_num(ip[0] as usize);
    console.print(".");
    console.print_num(ip[1] as usize);
    console.print(".");
    console.print_num(ip[2] as usize);
    console.print(".");
    console.print_num(ip[3] as usize);
    console.print("\n");
    console.print("  Глюз:  ");
    console.print_num(gw[0] as usize);
    console.print(".");
    console.print_num(gw[1] as usize);
    console.print(".");
    console.print_num(gw[2] as usize);
    console.print(".");
    console.print_num(gw[3] as usize);
    console.print("\n");
    console.print("  Маска: ");
    console.print_num(mask[0] as usize);
    console.print(".");
    console.print_num(mask[1] as usize);
    console.print(".");
    console.print_num(mask[2] as usize);
    console.print(".");
    console.print_num(mask[3] as usize);
    console.print("\n");
}

/// Тест ping
fn run_ping_test(console: &mut Console) {
    console.print_colored("\n--- Ping Test ---\n", COLOR_YELLOW);

    if !crate::net::is_initialized() {
        console.print_colored("  Сеть не инициализирована\n", COLOR_RED);
        return;
    }

    let gw = crate::net::get_gateway();
    console.print("  Пинг шлюза (");
    console.print_num(gw[0] as usize);
    console.print(".");
    console.print_num(gw[1] as usize);
    console.print(".");
    console.print_num(gw[2] as usize);
    console.print(".");
    console.print_num(gw[3] as usize);
    console.print(") ...\n");

    crate::net::icmp::ping(gw, 3);
}

/// Устройства USB
fn show_usb_devices(console: &mut Console) {
    console.print_colored("\n--- USB Devices ---\n", COLOR_YELLOW);

    let controllers = crate::drivers::usb::get_controllers();
    if controllers.is_empty() {
        console.print("  USB контроллеры не найдены\n");
        return;
    }

    console.print("  Найдено контроллеров: ");
    console.print_num(controllers.len());
    console.print("\n\n");

    for (i, ctrl) in controllers.iter().enumerate() {
        console.print("  [");
        console.print_num(i);
        console.print("] USB Controller\n");
        console.print("      Vendor:Device = ");
        print_hex4(console, ctrl.vendor);
        console.print(":");
        print_hex4(console, ctrl.device);
        console.print("\n");
        console.print("      Ports: ");
        console.print_num(ctrl.port_count as usize);
        console.print("\n");
    }
}

/// Статус APIC
fn show_apic_status(console: &mut Console) {
    console.print_colored("\n--- APIC Status ---\n", COLOR_YELLOW);
    let active = crate::interrupts::apic::is_active();
    let has_apic = crate::interrupts::apic::has_apic();

    console.print("  Обнаружен: ");
    console.print(if has_apic { "да" } else { "нет" });
    console.print("\n");
    console.print("  Активен:   ");
    console.print(if active { "да (полный APIC, PIC off)" } else { "нет (PIC основной)" });
    console.print("\n");

    if active {
        console.print("  Таймер:    LAPIC periodic 100Hz, vector 32\n");
        console.print("  Клавиатура: IO APIC GSI1 -> vector 33\n");
        console.print("  Serial:    polling (без IO APIC)\n");
    }
}

/// Статистика ядра
fn show_kernel_stats(console: &mut Console) {
    console.print_colored("\n--- Kernel Stats ---\n", COLOR_YELLOW);

    let ticks = crate::interrupts::idt::ticks();
    let kb_irq = crate::keyboard::irq_count();
    let seconds = ticks / 100;

    console.print("  Timer ticks:   ");
    console.print_num(ticks as usize);
    console.print("\n");
    console.print("  Uptime:        ");
    console.print_num((seconds / 3600) as usize);
    console.print("h ");
    console.print_num(((seconds / 60) % 60) as usize);
    console.print("m ");
    console.print_num((seconds % 60) as usize);
    console.print("s\n");
    console.print("  KB IRQ count:  ");
    console.print_num(kb_irq as usize);
    console.print("\n");

    let (sched_ticks, ctx_sw, _) = crate::task::stats();
    console.print("  Sched ticks:   ");
    console.print_num(sched_ticks as usize);
    console.print("\n");
    console.print("  Ctx switches:  ");
    console.print_num(ctx_sw as usize);
    console.print("\n");

    let (used_frames, total_frames) = crate::memory::pmm::stats();
    let (heap_used, heap_total) = crate::memory::heap::stats();
    console.print("  PMM frames:    ");
    console.print_num(used_frames);
    console.print(" / ");
    console.print_num(total_frames);
    console.print("\n");
    console.print("  Heap:          ");
    console.print_num(heap_used / 1024);
    console.print(" / ");
    console.print_num(heap_total / 1024);
    console.print(" KB\n");

    let task_count = crate::task::task_count();
    console.print("  Tasks:         ");
    console.print_num(task_count);
    console.print("\n");
}
