use super::CmdResult;
use crate::framebuffer::{Console, COLOR_YELLOW};

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() {
        console.print_colored("Usage: beep <freq_hz> [duration_ms]\n", COLOR_YELLOW);
        console.print("  beep 440        - play 440Hz (A4 note)\n");
        console.print("  beep 880 500    - play 880Hz for 500ms\n");
        console.print("  beep 0          - silence\n");
        return CmdResult::Ok;
    }

    let freq: u32 = args[0].parse().unwrap_or(0);
    let duration_ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);

    if freq == 0 {
        crate::devices::pc_speaker::silent();
        console.print("Silent\n");
    } else {
        crate::devices::pc_speaker::beep(freq);
        console.print("Playing ");
        console.print_num(freq as usize);
        console.print(" Hz for ");
        console.print_num(duration_ms as usize);
        console.print("ms\n");

        // Точная задержка через scheduler (1 тик = 10мс)
        let ticks = (duration_ms + 9) / 10;
        let current = crate::task::current_task_id().unwrap_or(0);
        let deadline = crate::interrupts::idt::TICKS.load(core::sync::atomic::Ordering::Relaxed) + ticks;
        while crate::interrupts::idt::TICKS.load(core::sync::atomic::Ordering::Relaxed) < deadline {
            core::hint::spin_loop();
        }
        crate::devices::pc_speaker::silent();
    }
    CmdResult::Ok
}
