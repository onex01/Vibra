// test — единая точка запуска системных проверок.
//
// Вместо запоминания `diag pmm`, `diag timer`, `diag usb`, ...
// команда группирует их: `test`, `test all`, `test cpu`, `test mem`, ...

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let what = args.first().copied().unwrap_or("all");

    match what {
        "all" => {
            section(console, "CPU");
            let _ = super::top::run(&["-b"], console);
            section(console, "Memory");
            let _ = super::diag::run(&["pmm"], console);
            let _ = super::diag::run(&["paging"], console);
            section(console, "PCI");
            let _ = super::lspci::run(&[], console);
            section(console, "Timer");
            let _ = super::diag::run(&["timer"], console);
            section(console, "USB");
            let _ = super::diag::run(&["usb"], console);
            section(console, "Keyboard");
            let _ = super::diag::run(&["keyboard"], console);
            console.print_colored("\n[TEST] all: done\n", COLOR_GREEN);
        }
        "cpu" => {
            let _ = super::top::run(&["-b"], console);
        }
        "mem" | "memory" => {
            let _ = super::diag::run(&["pmm"], console);
            let _ = super::diag::run(&["paging"], console);
            let _ = super::free::run(&[], console);
        }
        "heap" => {
            let _ = super::diag::run(&["heap-test"], console);
        }
        "pci" => {
            let _ = super::lspci::run(&[], console);
        }
        "disk" | "disks" => {
            let _ = super::df::run(&[], console);
            let _ = super::mount::run(&[], console);
        }
        "net" | "network" => {
            let _ = super::net::ifconfig(&[], console);
        }
        "usb" => {
            let _ = super::diag::run(&["usb"], console);
        }
        "timer" | "time" => {
            let _ = super::diag::run(&["timer"], console);
        }
        "keyboard" => {
            let _ = super::diag::run(&["keyboard"], console);
        }
        "power" => {
            let _ = super::diag::run(&["power"], console);
        }
        "hw" => {
            let _ = super::diag::run(&["hw"], console);
        }
        "-h" | "--help" | "help" => {
            console.print_colored("Usage: test [all|cpu|mem|heap|pci|disk|net|usb|timer|keyboard|power|hw]\n", COLOR_YELLOW);
        }
        other => {
            console.print_colored("test: unknown target '", COLOR_CYAN);
            console.print(other);
            console.print_colored("' (try `test help`)\n", COLOR_CYAN);
        }
    }

    CmdResult::Ok
}

fn section(console: &mut Console, name: &str) {
    console.print_colored("\n=== ", COLOR_CYAN);
    console.print_colored(name, COLOR_YELLOW);
    console.print_colored(" ===\n", COLOR_CYAN);
}