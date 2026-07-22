use vibra_kernel::commands::CmdResult;
use vibra_kernel::framebuffer::Console;

pub fn run(_args: &[&str], console: &mut Console) -> CmdResult {
    let info = vibra_kernel::cpu_info::detect();

    console.print_colored("CPU Information:\n", vibra_kernel::framebuffer::COLOR_CYAN);
    console.print("  Vendor:   ");
    console.print(vibra_kernel::cpu_info::vendor_str(&info));
    console.print("\n");

    console.print("  Brand:    ");
    console.print(vibra_kernel::cpu_info::brand_str(&info));
    console.print("\n");

    let freq = vibra_kernel::cpu_info::freq_str(&info);
    console.print("  Freq:     ");
    console.print(&freq);
    console.print("\n");

    console.print("  Family:   ");
    console.print_num(info.family as usize);
    console.print("  Model:    ");
    console.print_num(info.model as usize);
    console.print("  Stepping: ");
    console.print_num(info.stepping as usize);
    console.print("\n");

    console.print("  Cores:    ");
    console.print_num(info.cores as usize);
    console.print("  SMT:      ");
    if info.topology.smt { console.print("yes"); } else { console.print("no"); }
    console.print("\n");

    console.print_colored("\nFeatures:\n", vibra_kernel::framebuffer::COLOR_CYAN);
    let f = &info.features;
    let features_list = [
        ("FPU", f.fpu), ("SSE", f.sse), ("SSE2", f.sse2),
        ("SSE3", f.sse3), ("SSSE3", f.ssse3), ("SSE4.1", f.sse4_1),
        ("SSE4.2", f.sse4_2), ("AVX", f.avx), ("AVX2", f.avx2),
        ("NX", f.nx), ("APIC", f.apic), ("TSC", f.tsc),
        ("SMEP", f.smep), ("SMAP", f.smap), ("MMX", f.mmx),
        ("64-bit", f.lm), ("1GB pages", f.pdpe1gb),
        ("SYSCALL", f.syscall_sysret), ("BMI1", f.bmi1), ("BMI2", f.bmi2),
    ];
    for (name, present) in features_list {
        if present {
            console.print("  ");
            console.print_colored(name, vibra_kernel::framebuffer::COLOR_GREEN);
            console.print("  ");
        }
    }
    console.print("\n");

    CmdResult::Ok
}
