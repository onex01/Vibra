use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.iter().any(|a| *a == "-h" || *a == "--help") {
        console.print_colored("Usage: beep [freq_hz] [duration_ms]\n", COLOR_YELLOW);
        console.print("  beep            - short 1000 Hz beep\n");
        console.print("  beep 440        - play 440Hz (A4 note)\n");
        console.print("  beep 880 500    - play 880Hz for 500ms\n");
        console.print("  beep 0          - silence\n");
        return CmdResult::Ok;
    }

    // Без аргументов — короткий сигнал, как `bell` в терминале.
    let freq: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let duration: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200).min(10_000);

    if freq == 0 {
        crate::devices::pc_speaker::silent();
        console.print("Silent\n");
    } else {
        crate::devices::pc_speaker::beep(freq);
        console.print("Playing ");
        console.print_num(freq as usize);
        console.print(" Hz for ");
        console.print_num(duration as usize);
        console.print("ms (Ctrl+C to stop)\n");

        let start = crate::timer::uptime_ms();
        while crate::timer::uptime_ms().wrapping_sub(start) < duration as u64 {
            // Даём прервать длинный beep.
            if crate::keyboard::poll_key().is_some() || crate::is_cancelled() {
                crate::reset_cancel();
                break;
            }
            core::hint::spin_loop();
        }
        crate::devices::pc_speaker::silent();
    }
    CmdResult::Ok
}
