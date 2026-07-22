#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![deny(improper_ctypes)]

extern crate alloc;

mod serial;
mod gdt;
mod memory;
mod keyboard;
mod framebuffer;
mod fs;
mod commands;
mod shell;
mod kernel; 
mod version;
mod interrupts;
mod input;
mod devices;
mod task;
mod users;
mod cpu_info;
mod drivers;
mod syscall;
mod script;
mod boot_log;

use core::panic::PanicInfo;
use limine::request::{FramebufferRequest, HhdmRequest, MemmapRequest, ExecutableAddressRequest};
use spin::Mutex;
use commands::CmdResult;

#[used] static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();
#[used] static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();
#[used] static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();
#[used] static EXECUTABLE_ADDR_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[inline]
fn halt() { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }

// Стресс-тест heap: 10k циклов alloc/drop разного размера.
// used ДО и ПОСЛЕ должен совпасть — иначе free/коалесценция сломаны.
fn heap_stress() {
    use alloc::{vec, vec::Vec};
    const ITERS: usize = 10_000;

    let (used_before, _) = memory::heap::stats();

    // Паттерн длин 8..256 по модулю — дразним фрагментацию.
    let mut patterns = [0u8; 256];
    let mut seed: u8 = 0x5A;
    for i in 0..256 {
        seed ^= (i as u8).wrapping_mul(0x9B);
        patterns[i] = seed;
    }

    for i in 0..ITERS {
        let n = 8 + (i % 248); // 8..255
        let mut v: Vec<u8> = Vec::with_capacity(n);
        // Заполняем реальными данными, чтобы словить порчу метаданных.
        let mut idx = i;
        while v.len() < n {
            v.push(patterns[idx % 256]);
            idx = idx.wrapping_add(7);
        }
        // Проверка целостности перед drop (ловим запись в чужой блок).
        let sample = v[n / 2];
        let expected = patterns[(i + (n / 2) * 7) % 256];
        if sample != expected {
            println!("[HEAP] stress: CORRUPTION at iter {} (got {:#x} want {:#x})", i, sample, expected);
            return;
        }
        // drop тут же — RAII освобождает блок.
        drop(v);

        // Раз в ~2000 итераций держим несколько живых блоков одновременно,
        // чтобы список реально фрагментировался, а не бегал одним узлом.
        if i % 2000 == 0 && i != 0 {
            let held_len = 8 + ((i / 2000) * 37) % 248;
            let _h = vec![0xCDu8; held_len]; // живёт до конца итерации блока
            let _ = _h;
        }
    }

    let (used_after, _) = memory::heap::stats();
    if used_after == used_before {
        println!("[HEAP] stress: {} iters OK (used {} -> {})", ITERS, used_before, used_after);
    } else {
        let drift = used_after as i64 - used_before as i64;
        println!("[HEAP] stress: {} iters LEAK {} ({} -> {})", ITERS, drift, used_before, used_after);
    }
}

// LineEditor хранит историю и буфер ввода между командами. Доступ идёт только
// из основного потока; Mutex убирает небезопасную mutable-ссылку на static и
// оставляет корректную точку синхронизации для будущего планировщика.
static LINE_EDITOR: Mutex<shell::LineEditor> = Mutex::new(shell::LineEditor::new());

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial::init();
    println!(
        "Vibra {} \"{}\" booting...",
        version::OS_VERSION,
        version::OS_CODENAME
    );

    let hhdm_response = match HHDM_REQUEST.response() {
        Some(r) => r,
        None => { println!("[FATAL] HHDM failed"); loop { halt(); } }
    };
    let hhdm_offset = hhdm_response.offset;

    if let Some(mm) = MEMORY_MAP_REQUEST.response() {
        memory::init(mm.entries(), hhdm_offset);
    } else { println!("[FATAL] Memory map failed"); loop { halt(); } }

    // Heap stress: 10k alloc/drop разной длины. Доказывает корректность
    // free-list'а: used должен вернуться к базовому (показывает работу free
    // и коалесценции). Делаем до kernel::init — там первые постоянные
    // аллокации (Vec/String), которые не освобождаются.
    heap_stress();

    keyboard::init();

    // Подсистема ввода (унифицированные события)
    input::init();

    // Виртуальные устройства
    devices::init();
    devices::virtio_block::probe_devices();

    // PCI + AHCI/SATA драйверы (для реального железа)
    drivers::init();
    
    // Инициализация файловых систем
    fs::init_filesystem();
    
    // Сохраняем лог загрузки в файл
    boot_log::init();
    
    // Mount VFS (пока только ramfs)
    use alloc::boxed::Box;
    use crate::fs::{FileSystem, VfsManager};
    let mut vfs = VfsManager::new();
    let mut ramfs = fs::RamFs::new();
    FileSystem::mount(&mut ramfs).ok();
    vfs.mount("/", Box::new(ramfs), false).ok();
    
    kernel::init();

    // Планировщик задач (заглушка)
    task::init();

    // Система пользователей
    users::init();
    
    // Регистрация устройств
    kernel::device::register("console", kernel::device::DeviceType::Console);
    kernel::device::register("keyboard", kernel::device::DeviceType::Keyboard);
    kernel::device::register("framebuffer", kernel::device::DeviceType::Display);
    kernel::device::register("ramfs", kernel::device::DeviceType::Disk);
    kernel::device::register("pit-timer", kernel::device::DeviceType::Timer);
    
    kernel::driver::register("ps2-keyboard", "0.1.0", &[kernel::device::DeviceType::Keyboard]);
    kernel::driver::register("vga-console", "0.1.0", &[kernel::device::DeviceType::Console]);
    kernel::driver::register("fbdev", "0.1.0", &[kernel::device::DeviceType::Display]);
    
    // === VMM: Построить и активировать собственные page tables ===
    // Получаем адреса ядра от Limine
    let exec_addr_response = EXECUTABLE_ADDR_REQUEST.response();
    let (kernel_phys_base, kernel_virt_base) = match exec_addr_response {
        Some(addr) => (addr.physical_base, addr.virtual_base),
        None => {
            println!("[VMM] WARNING: ExecutableAddressRequest failed, using defaults");
            (0xffffffff80000000u64, 0xffffffff80000000u64) // fallback
        }
    };
    println!("[VMM] Kernel phys={:#x}, virt={:#x}", kernel_phys_base, kernel_virt_base);

    // Получаем framebuffer — fb.address() возвращает виртуальный адрес через HHDM
    let (fb_virt, fb_size) = match FRAMEBUFFER_REQUEST.response() {
        Some(fb_resp) => match fb_resp.framebuffers().first() {
            Some(fb) => (fb.address() as u64, (fb.pitch as u64) * fb.height),
            None => (0, 0),
        },
        None => (0, 0),
    };
    let fb_phys = if fb_virt != 0 { fb_virt - hhdm_offset } else { 0 };
    println!("[VMM] Framebuffer virt={:#x} phys={:#x}, size={:#x}", fb_virt, fb_phys, fb_size);

    // Получаем memory map
    let memory_map_entries = match MEMORY_MAP_REQUEST.response() {
        Some(mm) => mm.entries(),
        None => { println!("[FATAL] Memory map failed for VMM"); loop { halt(); } }
    };

    // Инициализация VMM: построить новые page tables и активировать
    match memory::vmm::init(
        memory_map_entries,
        hhdm_offset,
        kernel_phys_base,
        kernel_virt_base,
        fb_phys,
        fb_size,
    ) {
        Some(_pml4_phys) => {
            println!("[VMM] Initialization complete!");
        }
        None => {
            println!("[VMM] FATAL: Failed to initialize VMM!");
            loop { halt(); }
        }
    }

    // GDT и IDT строим ПОСЛЕ переключения page tables
    gdt::init();
    interrupts::init();

    // syscall/sysret MSR setup (нужен после gdt::init, до sti)
    syscall::init();

    // APIC: LAPIC init + IO APIC masked (PIC остаётся primary).
    // APIC: LAPIC init + IO APIC masked (PIC остаётся primary).
    // Assembly MMIO fix: теперь LAPIC MMIO writes НЕ ломают serial.
    // crate::interrupts::apic::init();

    interrupts::enable();

    // Повторная инициализация PS/2 после APIC takeover (IO APIC для IRQ1)
    crate::keyboard::post_init();

    println!("[DEBUG] Interrupts enabled, continuing boot...");
    println!("[DEBUG] About to draw ASCII art...");

    let mut console = match FRAMEBUFFER_REQUEST.response() {
        Some(fb_resp) => match fb_resp.framebuffers().first() {
            Some(fb) => framebuffer::Console::new(fb),
            None => { println!("[FATAL] No framebuffer"); loop { halt(); } }
        },
        None => { println!("[FATAL] Framebuffer request failed"); loop { halt(); } }
    };

    // Приветствие
    console.print_colored("\n", framebuffer::COLOR_VIBRA_FG);
    console.print_colored("     __     ___ _           \n", framebuffer::COLOR_CYAN);
    console.print_colored("     \\ \\   / (_) |__  _ __ __ _ \n", framebuffer::COLOR_CYAN);
    console.print_colored("      \\ \\ / /| | '_ \\| '__/ _` |\n", framebuffer::COLOR_CYAN);
    console.print_colored("       \\ V / | | |_) | | | (_| |\n", framebuffer::COLOR_CYAN);
    console.print_colored("        \\_/  |_|_.__/|_|  \\__,_|\n", framebuffer::COLOR_CYAN);
    console.print("\n");
    console.print_colored("    Vibra OS v", framebuffer::COLOR_VIBRA_PROMPT);
    console.print_colored(version::OS_VERSION, framebuffer::COLOR_VIBRA_PROMPT);
    console.print_colored(" \"", framebuffer::COLOR_VIBRA_PROMPT);
    console.print_colored(version::OS_CODENAME, framebuffer::COLOR_VIBRA_PROMPT);
    console.print_colored("\"\n", framebuffer::COLOR_VIBRA_PROMPT);
    console.print_colored("    Kernel v", framebuffer::COLOR_VIBRA_FG);
    console.print_colored(version::KERNEL_VERSION, framebuffer::COLOR_VIBRA_FG);
    console.print_colored(" \"", framebuffer::COLOR_VIBRA_FG);
    console.print_colored(version::KERNEL_CODENAME, framebuffer::COLOR_VIBRA_FG);
    console.print_colored("\"\n", framebuffer::COLOR_VIBRA_FG);
    console.print_colored("    Type 'help' for commands | Tab to autocomplete\n\n", framebuffer::COLOR_VIBRA_FG);

    // Raw marker: пишем 'C' через port I/O перед циклом shell
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'C', options(nostack, preserves_flags));
    }

    loop {
        // Динамический prompt: vibra:/path>
        let current_dir = fs::get_current_dir();
        let mut prompt_buf = [0u8; 128];
        let prompt_str = {
            let mut pos = 0;
            for b in b"vibra:" {
                if pos < prompt_buf.len() { prompt_buf[pos] = *b; pos += 1; }
            }
            for b in current_dir.as_bytes() {
                if pos < prompt_buf.len() { prompt_buf[pos] = *b; pos += 1; }
            }
            for b in b"> " {
                if pos < prompt_buf.len() { prompt_buf[pos] = *b; pos += 1; }
            }
            core::str::from_utf8(&prompt_buf[..pos]).unwrap_or("vibra> ")
        };
        let prompt_len = prompt_str.len();

        console.print_colored(prompt_str, framebuffer::COLOR_VIBRA_PROMPT);

        // Копируем prompt в буфер редактора для tab completion
        let mut line_buffer = [0u8; 256];
        let line_len = {
            let mut editor = LINE_EDITOR.lock();
            // Копируем prompt в prompt_buf редактора
            editor.set_prompt(prompt_str);
            let line = editor.read_line(&mut console, prompt_len);
            line_buffer[..line.len()].copy_from_slice(line.as_bytes());
            line.len()
        };
        let line = core::str::from_utf8(&line_buffer[..line_len]).unwrap_or("");
        let trimmed = line.trim();

        if trimmed.is_empty() { continue; }

        // Парсим команду и аргументы
        let mut parts: [&str; 16] = [""; 16];
        let mut n_parts = 0usize;
        for p in trimmed.split_whitespace() {
            if n_parts < 16 { parts[n_parts] = p; n_parts += 1; }
        }

        if n_parts == 0 { continue; }

        let cmd_name = parts[0];
        let args = &parts[1..n_parts];

        if let Some(cmd) = commands::find_command(cmd_name) {
            match (cmd.func)(args, &mut console) {
                CmdResult::Ok | CmdResult::Continue => {}
                CmdResult::Exit => {
                    console.print_colored("System halting...\n", framebuffer::COLOR_RED);
                    loop { halt(); }
                }
            }
        } else {
            console.print_colored("Unknown command: '", framebuffer::COLOR_RED);
            console.print(cmd_name);
            console.print_colored("'. Type 'help'.\n", framebuffer::COLOR_RED);
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n!!! KERNEL PANIC !!!");
    println!("{}", info);
    // Пытаемся сохранить лог перед зависанием
    boot_log::flush_to_file("/var/log/error.log");
    loop { halt(); }
}
