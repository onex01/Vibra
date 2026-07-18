#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

#![feature(cmse_nonsecure_entry)]
#![deny(improper_ctypes)]

extern crate alloc;

mod serial;
mod memory;
mod keyboard;
mod framebuffer;
mod fs;
mod commands;
mod shell;
mod kernel; 
mod version;
mod interrupts;

use core::panic::PanicInfo;
use limine::request::{FramebufferRequest, HhdmRequest, MemmapRequest};
use commands::CmdResult;

#[used] static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();
#[used] static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();
#[used] static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

const OS_VERSION: &str = "0.4";
const OS_CODENAME: &str = "Photon";

#[inline]
fn halt() { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }

static mut LINE_EDITOR: shell::LineEditor = shell::LineEditor::new();

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial::init();
    println!("Vibra {} \"{}\" booting...", OS_VERSION, OS_CODENAME);

    let hhdm_response = match HHDM_REQUEST.response() {
        Some(r) => r,
        None => { println!("[FATAL] HHDM failed"); loop { halt(); } }
    };
    let hhdm_offset = hhdm_response.offset;

    if let Some(mm) = MEMORY_MAP_REQUEST.response() {
        memory::init(mm.entries());
    } else { println!("[FATAL] Memory map failed"); loop { halt(); } }

    // Memory test
    if let Some(frame) = memory::pmm::alloc_frame() {
        let virt = (frame + hhdm_offset as usize) as *mut u8;
        unsafe { core::ptr::write_volatile(virt, 0xAB); }
        memory::pmm::free_frame(frame);
    }

    keyboard::init();
    fs::init_filesystem();
    kernel::init();
    
    // Регистрация устройств
    kernel::device::register("console", kernel::device::DeviceType::Console);
    kernel::device::register("keyboard", kernel::device::DeviceType::Keyboard);
    kernel::device::register("framebuffer", kernel::device::DeviceType::Display);
    kernel::device::register("ramfs", kernel::device::DeviceType::Disk);
    kernel::device::register("pit-timer", kernel::device::DeviceType::Timer);
    
    kernel::driver::register("ps2-keyboard", "0.1.0", &[kernel::device::DeviceType::Keyboard]);
    kernel::driver::register("vga-console", "0.1.0", &[kernel::device::DeviceType::Console]);
    kernel::driver::register("fbdev", "0.1.0", &[kernel::device::DeviceType::Display]);
    
    interrupts::init();
    interrupts::enable();

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

    loop {
        let prompt = "vibra> ";
        console.print_colored(prompt, framebuffer::COLOR_VIBRA_PROMPT);
        let prompt_len = prompt.len();
        let line = unsafe { LINE_EDITOR.read_line(&mut console, prompt_len) };
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
    loop { halt(); }
}