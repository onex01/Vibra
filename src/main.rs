#![no_std]
#![no_main]

mod serial;
mod memory;
mod keyboard;
mod framebuffer;
mod fs;
mod commands;
mod shell;

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

    let mut console = match FRAMEBUFFER_REQUEST.response() {
        Some(fb_resp) => match fb_resp.framebuffers().first() {
            Some(fb) => framebuffer::Console::new(fb),
            None => { println!("[FATAL] No framebuffer"); loop { halt(); } }
        },
        None => { println!("[FATAL] Framebuffer request failed"); loop { halt(); } }
    };

    // Приветствие
    console.print_colored("\n     __     ___ _           \n", framebuffer::COLOR_CYAN);
    console.print_colored("     \\ \\   / (_) |__  _ __ __ _ \n", framebuffer::COLOR_CYAN);
    console.print_colored("      \\ \\ / /| | '_ \\| '__/ _` |\n", framebuffer::COLOR_CYAN);
    console.print_colored("       \\ V / | | |_) | | | (_| |\n", framebuffer::COLOR_CYAN);
    console.print_colored("        \\_/  |_|_.__/|_|  \\__,_|\n", framebuffer::COLOR_CYAN);
    console.print("\n");
    console.print_colored("    Vibra OS v0.4 \"Photon\"\n", framebuffer::COLOR_VIBRA_PROMPT);
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