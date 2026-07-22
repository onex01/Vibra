#![no_std]
#![feature(abi_x86_interrupt)]
#![deny(improper_ctypes)]

extern crate alloc;

// === Публичные модули ядра ===
pub mod serial;
pub mod gdt;
pub mod memory;
pub mod keyboard;
pub mod framebuffer;
pub mod fs;
pub mod commands;
pub mod shell;
pub mod kernel;
pub mod version;
pub mod interrupts;
pub mod input;
pub mod devices;
pub mod task;
pub mod users;
pub mod cpu_info;
pub mod drivers;
pub mod syscall;
pub mod script;
pub mod boot_log;
pub mod graphics;

// === Limine requests (нужны в lib crate для линкера) ===
use limine::request::{FramebufferRequest, HhdmRequest, MemmapRequest, ExecutableAddressRequest};
use spin::Mutex;
use commands::CmdResult;

use core::sync::atomic::{AtomicBool, Ordering};

pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();
pub static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();
pub static EXECUTABLE_ADDR_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

/// Флаг отмены текущей команды (Ctrl+Z)
pub static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

/// Проверить, запросил ли пользователь отмену
pub fn is_cancelled() -> bool {
    CANCEL_FLAG.load(Ordering::Relaxed)
}

/// Сбросить флаг отмены
pub fn reset_cancel() {
    CANCEL_FLAG.store(false, Ordering::Relaxed);
}

/// Установить флаг отмены
pub fn request_cancel() {
    CANCEL_FLAG.store(true, Ordering::Relaxed);
}

#[inline]
fn halt() { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }

// Стресс-тест heap: 10k циклов alloc/drop разного размера.
fn heap_stress() {
    use alloc::{vec, vec::Vec};
    const ITERS: usize = 10_000;

    let (used_before, _) = memory::heap::stats();

    let mut patterns = [0u8; 256];
    let mut seed: u8 = 0x5A;
    for i in 0..256 {
        seed ^= (i as u8).wrapping_mul(0x9B);
        patterns[i] = seed;
    }

    for i in 0..ITERS {
        let n = 8 + (i % 248);
        let mut v: Vec<u8> = Vec::with_capacity(n);
        let mut idx = i;
        while v.len() < n {
            v.push(patterns[idx % 256]);
            idx = idx.wrapping_add(7);
        }
        let sample = v[n / 2];
        let expected = patterns[(i + (n / 2) * 7) % 256];
        if sample != expected {
            println!("[HEAP] stress: CORRUPTION at iter {} (got {:#x} want {:#x})", i, sample, expected);
            return;
        }
        drop(v);

        if i % 2000 == 0 && i != 0 {
            let held_len = 8 + ((i / 2000) * 37) % 248;
            let _h = vec![0xCDu8; held_len];
            let _ = _h;
        }
    }

    let (used_after, _) = memory::heap::stats();
    if used_after == used_before {
        println!("[HEAP] stress: {} iters OK (used {} -> {})", ITERS, used_before, used_after);
    } else {
        let drift = used_after as i64 - used_before as i64;
        println!("[HEAP] stress: {} iters LEAK ({} -> {})", ITERS, used_before, used_after);
    }
}

/// Публичные типы для shell loop (используются vibra)
pub struct BootConsole {
    pub console: framebuffer::Console,
}

/// Точка входа ядра: полная инициализация + shell.
/// Вызывается из _start() бинарника.
pub fn boot() -> ! {
    let bc = init();
    shell_loop(bc);
}

/// Инициализация ядра. Возвращает BootConsole с готовой framebuffer консолью.
/// Вызывается vibra для запуска своего shell loop.
pub fn init() -> BootConsole {
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

    heap_stress();

    keyboard::init();
    input::init();
    devices::init();
    devices::virtio_block::probe_devices();
    drivers::init();
    fs::init_filesystem();
    boot_log::init();

    use alloc::boxed::Box;
    use crate::fs::{FileSystem, VfsManager};
    let mut vfs = VfsManager::new();
    let mut ramfs = fs::RamFs::new();
    FileSystem::mount(&mut ramfs).ok();
    vfs.mount("/", Box::new(ramfs), false).ok();

    kernel::init();
    task::init();
    users::init();

    kernel::device::register("console", kernel::device::DeviceType::Console);
    kernel::device::register("keyboard", kernel::device::DeviceType::Keyboard);
    kernel::device::register("framebuffer", kernel::device::DeviceType::Display);
    kernel::device::register("ramfs", kernel::device::DeviceType::Disk);
    kernel::device::register("pit-timer", kernel::device::DeviceType::Timer);

    kernel::driver::register("ps2-keyboard", "0.1.0", &[kernel::device::DeviceType::Keyboard]);
    kernel::driver::register("vga-console", "0.1.0", &[kernel::device::DeviceType::Console]);
    kernel::driver::register("fbdev", "0.1.0", &[kernel::device::DeviceType::Display]);

    let exec_addr_response = EXECUTABLE_ADDR_REQUEST.response();
    let (kernel_phys_base, kernel_virt_base) = match exec_addr_response {
        Some(addr) => (addr.physical_base, addr.virtual_base),
        None => {
            println!("[VMM] WARNING: ExecutableAddressRequest failed, using defaults");
            (0xffffffff80000000u64, 0xffffffff80000000u64)
        }
    };

    let (fb_virt, fb_size) = match FRAMEBUFFER_REQUEST.response() {
        Some(fb_resp) => match fb_resp.framebuffers().first() {
            Some(fb) => (fb.address() as u64, (fb.pitch as u64) * fb.height),
            None => (0, 0),
        },
        None => (0, 0),
    };
    let fb_phys = if fb_virt != 0 { fb_virt - hhdm_offset } else { 0 };

    let memory_map_entries = match MEMORY_MAP_REQUEST.response() {
        Some(mm) => mm.entries(),
        None => { println!("[FATAL] Memory map failed for VMM"); loop { halt(); } }
    };

    match memory::vmm::init(
        memory_map_entries, hhdm_offset, kernel_phys_base, kernel_virt_base, fb_phys, fb_size,
    ) {
        Some(_pml4_phys) => println!("[VMM] Initialization complete!"),
        None => { println!("[VMM] FATAL: Failed to initialize VMM!"); loop { halt(); } }
    }

    gdt::init();
    interrupts::init();
    syscall::init();
    interrupts::enable();
    keyboard::post_init();

    println!("[DEBUG] Interrupts enabled, continuing boot...");

    let console = match FRAMEBUFFER_REQUEST.response() {
        Some(fb_resp) => match fb_resp.framebuffers().first() {
            Some(fb) => framebuffer::Console::new(fb),
            None => { println!("[FATAL] No framebuffer"); loop { halt(); } }
        },
        None => { println!("[FATAL] Framebuffer request failed"); loop { halt(); } }
    };

    BootConsole { console }
}

/// Shell loop — используется и kernel::boot() и vibra.
/// Vibra может вызвать init() + shell_loop() или написать свой loop.
pub fn shell_loop(mut bc: BootConsole) -> ! {
    let mut console = bc.console;

    console.print_colored("\n", framebuffer::COLOR_VIBRA_FG);
    console.print_colored("     __     ___ _           \n", framebuffer::COLOR_CYAN);
    console.print_colored("     \\ \\   / (_) |__  _ __ __ _ \n", framebuffer::COLOR_CYAN);
    console.print_colored("      \\ \\ / /| | '_ \\| '__/ _` |\n", framebuffer::COLOR_CYAN);
    console.print_colored("       \\ V / | | |_) | | | (_| |\n", framebuffer::COLOR_CYAN);
    console.print_colored("        \\_/  |_|_.__/|_|  \\__,_|\n", framebuffer::COLOR_CYAN);
    console.print("\n");
    console.print_colored("    Kernel v", framebuffer::COLOR_VIBRA_FG);
    console.print_colored(version::KERNEL_VERSION, framebuffer::COLOR_VIBRA_FG);
    console.print_colored(" \"", framebuffer::COLOR_VIBRA_FG);
    console.print_colored(version::KERNEL_CODENAME, framebuffer::COLOR_VIBRA_FG);
    console.print_colored("\"\n", framebuffer::COLOR_VIBRA_FG);
    console.print_colored("    Type 'help' for commands | Tab to autocomplete\n\n", framebuffer::COLOR_VIBRA_FG);

    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'C', options(nostack, preserves_flags));
    }

    static LINE_EDITOR: Mutex<shell::LineEditor> = Mutex::new(shell::LineEditor::new());

    loop {
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

        let mut line_buffer = [0u8; 256];
        let line_len = {
            let mut editor = LINE_EDITOR.lock();
            editor.set_prompt(prompt_str);
            let line = editor.read_line(&mut console, prompt_len);
            line_buffer[..line.len()].copy_from_slice(line.as_bytes());
            line.len()
        };
        let line = core::str::from_utf8(&line_buffer[..line_len]).unwrap_or("");
        let trimmed = line.trim();

        if trimmed.is_empty() { continue; }

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
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("\n!!! KERNEL PANIC !!!");
    println!("{}", info);
    boot_log::flush_to_file("/var/log/error.log");
    loop { halt(); }
}
