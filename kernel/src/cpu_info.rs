// CPU info — чтение информации о процессоре через CPUID.

use core::arch::asm;

pub struct CpuInfo {
    pub brand: [u8; 48],
    pub brand_len: usize,
    pub cores: u32,
    pub max_leaf: u32,
    pub tsc_freq_mhz: u64,
}

/// CPUID helper — сохраняем/восстанавливаем rbx вручную (LLVM его использует).
/// Возвращает (eax, ebx, ecx, edx)
unsafe fn cpuid_raw(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    let mut out = [0u32; 4]; // [eax, ebx, ecx, edx]
    asm!(
        "push rbx",
        "cpuid",
        "mov [{out} + 0], eax",
        "mov [{out} + 4], ebx",
        "mov [{out} + 8], ecx",
        "mov [{out} + 12], edx",
        "pop rbx",
        in("eax") leaf,
        in("ecx") sub,
        out = in(reg) out.as_mut_ptr(),
        options(nostack, nomem),
    );
    (out[0], out[1], out[2], out[3])
}

/// RDMSR для TSC frequency (platform info MSR 0xCE)
unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi);
    ((hi as u64) << 32) | (lo as u64)
}

pub fn detect() -> CpuInfo {
    let mut info = CpuInfo {
        brand: [0u8; 48],
        brand_len: 0,
        cores: 1,
        max_leaf: 0,
        tsc_freq_mhz: 0,
    };

    unsafe {
        // Max CPUID leaf
        let (max_leaf, _, _, _) = cpuid_raw(0, 0);
        info.max_leaf = max_leaf;

        // Brand string (leaves 0x80000002..0x80000004)
        if max_leaf >= 0x80000004 {
            let mut brand = [0u32; 12];
            for i in 0..3u32 {
                let (a, b, c, d) = cpuid_raw(0x80000002 + i, 0);
                brand[(i * 4) as usize] = a;
                brand[(i * 4 + 1) as usize] = b;
                brand[(i * 4 + 2) as usize] = c;
                brand[(i * 4 + 3) as usize] = d;
            }
            let brand_bytes: &[u8; 48] = core::slice::from_raw_parts(
                brand.as_ptr() as *const u8, 48
            ).try_into().unwrap();
            info.brand = *brand_bytes;
            // Находим конец строки (первый 0 байт)
            info.brand_len = info.brand.iter().position(|&b| b == 0).unwrap_or(48);
        }

        // Logical processors (leaf 0x01: EBX[23:16])
        if max_leaf >= 1 {
            let (_, ebx, _, _) = cpuid_raw(1, 0);
            info.cores = ((ebx >> 16) & 0xFF) as u32;
            if info.cores == 0 { info.cores = 1; }
        }

        // TSC frequency через platform info MSR (0xCE)
        // MSR 0xCE: bit 15:8 = bus frequency, bit 23:16 = TSC ratio
        // freq_mhz = ratio * bus_freq
        let platform_info = rdmsr(0xCE);
        let ratio = (platform_info >> 8) & 0xFF;
        let bus_freq = (platform_info >> 16) & 0xFF;
        if ratio > 0 && bus_freq > 0 {
            // bus_freq in MHz, ratio is multiplier
            info.tsc_freq_mhz = (ratio as u64) * (bus_freq as u64);
        } else {
            // Fallback: оценка ~2.4GHz для QEMU
            info.tsc_freq_mhz = 2400;
        }
    }

    info
}

/// Вернуть строку частоты: "2400 MHz" или "2.40 GHz"
pub fn freq_str(info: &CpuInfo) -> alloc::string::String {
    if info.tsc_freq_mhz >= 1000 {
        let whole = info.tsc_freq_mhz / 1000;
        let frac = (info.tsc_freq_mhz % 1000) / 10;
        alloc::format!("{}.{:02} GHz", whole, frac)
    } else {
        alloc::format!("{} MHz", info.tsc_freq_mhz)
    }
}

/// Вернуть строку имени процессора
pub fn brand_str(info: &CpuInfo) -> &str {
    let s = core::str::from_utf8(&info.brand[..info.brand_len]).unwrap_or("");
    if s.is_empty() {
        // QEMU не поддерживает CPUID brand — fallback
        "QEMU Virtual CPU"
    } else {
        s
    }
}
