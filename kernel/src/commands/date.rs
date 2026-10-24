// date — текущее время.
//
// Источник: время BIOS (CMOS RTC), зафиксированное при старте, плюс
// монотонные часы ядра. Часовой пояс прошивка не сообщает, поэтому время
// печатается как есть (обычно это локальное время машины).
//
//   date        — дата/время
//   date utc    — то же (совместимость с привычным флагом; сдвиг не применяется)
//   date hw     — «сырые» регистры CMOS и декодированное значение
//   date mono   — монотонный uptime в миллисекундах

use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW, COLOR_RED};
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    match args.first().copied() {
        None | Some("utc") | Some("-u") => show_clock(console),
        Some("hw") => show_hardware(console),
        Some("mono") => show_monotonic(console),
        Some(other) => {
            console.print_colored("date: unknown argument '", COLOR_RED);
            console.print(other);
            console.print_colored("'. Use: date | date hw | date mono\n", COLOR_RED);
        }
    }
    CmdResult::Ok
}

fn show_clock(console: &mut Console) {
    match crate::timer::wall_clock() {
        Some(dt) => {
            console.print(&format!("{}\n", dt.format()));
            console.print_colored("  source: ", COLOR_YELLOW);
            console.print(&format!(
                "RTC (BIOS) + {} ({} Hz)\n",
                crate::timer::current_source().name(),
                crate::timer::source_hz()
            ));
        }
        None => {
            console.print_colored("date: RTC unavailable, wall clock is not set\n", COLOR_RED);
            show_monotonic(console);
        }
    }
}

fn show_hardware(console: &mut Console) {
    console.print_colored("RTC (CMOS) state:\n", COLOR_YELLOW);
    match crate::timer::rtc::read_raw() {
        Some(raw) => {
            console.print(&format!(
                "  regs: sec={:#04x} min={:#04x} hour={:#04x} day={:#04x} mon={:#04x} year={:#04x} century={:#04x}\n",
                raw.second, raw.minute, raw.hour, raw.day, raw.month, raw.year, raw.century
            ));
            console.print(&format!(
                "  status B={:#04x}: {} , {} hour\n",
                raw.status_b,
                if raw.status_b & 0x04 == 0 { "BCD" } else { "binary" },
                if raw.status_b & 0x02 != 0 { "24" } else { "12" }
            ));
            match crate::timer::rtc::decode(&raw) {
                Some(t) => console.print(&format!(
                    "  decoded: {:04}-{:02}-{:02} {:02}:{:02}:{:02} (unix {})\n",
                    t.year, t.month, t.day, t.hour, t.minute, t.second, t.to_unix()
                )),
                None => console.print_colored("  decoded: invalid values\n", COLOR_RED),
            }
        }
        None => console.print_colored("  CMOS does not respond (UIP stuck or no RTC)\n", COLOR_RED),
    }

    console.print_colored("Kernel clock:\n", COLOR_YELLOW);
    console.print(&format!(
        "  boot RTC unix: {}\n  uptime:       {} ms\n",
        crate::timer::rtc::boot_unix(),
        crate::timer::uptime_ms()
    ));
}

fn show_monotonic(console: &mut Console) {
    let ns = crate::timer::uptime_ns();
    console.print_colored("Monotonic uptime: ", COLOR_YELLOW);
    console.print(&format!(
        "{} ms ({} s) via {}\n",
        ns / 1_000_000,
        ns / 1_000_000_000,
        crate::timer::current_source().name()
    ));
}
