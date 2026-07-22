// CPU info — чтение информации о процессоре через CPUID.
// Расширенное: feature flags, vendor, topology, cache info.

use core::arch::asm;

/// CPU feature flags (leaf 0x01 ECX/EDX + leaf 0x07 + leaf 0x80000001)
#[derive(Clone, Copy, Debug)]
pub struct CpuFeatures {
    // Leaf 0x01 EDX
    pub sse: bool,
    pub sse2: bool,
    pub fpu: bool,
    pub tsc: bool,
    pub msr: bool,
    pub apic: bool,
    pub cx8: bool,
    pub nx: bool,
    pub pse: bool,
    pub pge: bool,
    pub pat: bool,
    pub acpi: bool,
    pub mmx: bool,
    pub fxsave_fxstor: bool,
    // Leaf 0x01 ECX
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub avx: bool,
    pub f16c: bool,
    pub popcnt: bool,
    pub xsave: bool,
    pub osxsave: bool,
    // Leaf 0x07 EBX
    pub avx2: bool,
    pub bmi1: bool,
    pub bmi2: bool,
    pub smep: bool,
    pub smap: bool,
    // Leaf 0x80000001 EDX
    pub lm: bool,     // long mode (64-bit)
    pub syscall_sysret: bool,
    // Leaf 0x80000001 ECX
    pub pdpe1gb: bool, // 1GB huge pages
}

impl CpuFeatures {
    const fn empty() -> Self {
        Self {
            sse: false, sse2: false, fpu: false,
            tsc: false, msr: false, apic: false, cx8: false,
            nx: false, pse: false, pge: false, pat: false,
            acpi: false, mmx: false, fxsave_fxstor: false,
            sse3: false, ssse3: false, sse4_1: false, sse4_2: false,
            avx: false, f16c: false, popcnt: false, xsave: false,
            osxsave: false,
            avx2: false, bmi1: false, bmi2: false, smep: false, smap: false,
            lm: false, syscall_sysret: false,
            pdpe1gb: false,
        }
    }
}

/// Vendor ID процессора
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

/// Topology ядра (cores/threads)
#[derive(Clone, Copy, Debug)]
pub struct CpuTopology {
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub apic_id: u32,
    pub smt: bool,
}

pub struct CpuInfo {
    pub brand: [u8; 48],
    pub brand_len: usize,
    pub cores: u32,
    pub max_leaf: u32,
    pub max_ext_leaf: u32,
    pub tsc_freq_mhz: u64,
    pub vendor: CpuVendor,
    pub features: CpuFeatures,
    pub topology: CpuTopology,
    pub stepping: u32,
    pub model: u32,
    pub family: u32,
}

/// CPUID helper — сохраняем/восстанавливаем rbx вручную (LLVM его использует).
/// Возвращает (eax, ebx, ecx, edx)
unsafe fn cpuid_raw(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    let mut out = [0u32; 4];
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

/// RDMSR
unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi);
    ((hi as u64) << 32) | (lo as u64)
}

/// Определить vendor из CPUID leaf 0x00
fn detect_vendor(ebx: u32, ecx: u32, edx: u32) -> CpuVendor {
    // "GenuineIntel" = Genu ineI ntel
    if ebx == 0x756E6547 && ecx == 0x6C65746E && edx == 0x49656E69 {
        return CpuVendor::Intel;
    }
    // "AuthenticAMD" = Auth enti AMD
    if ebx == 0x68747541 && ecx == 0x444D4163 && edx == 0x69746E65 {
        return CpuVendor::Amd;
    }
    CpuVendor::Unknown
}

/// Определить CPU family/model/stepping из leaf 0x01
fn detect_family_model_stepping(eax: u32) -> (u32, u32, u32) {
    let stepping = eax & 0xF;
    let mut model = (eax >> 4) & 0xF;
    let mut family = (eax >> 8) & 0xF;

    if family == 0xF {
        family += (eax >> 20) & 0xFF;
    }
    if family == 0x6 || family == 0xF {
        model += ((eax >> 16) & 0xF) << 4;
    }

    (family, model, stepping)
}

pub fn detect() -> CpuInfo {
    let mut info = CpuInfo {
        brand: [0u8; 48],
        brand_len: 0,
        cores: 1,
        max_leaf: 0,
        max_ext_leaf: 0,
        tsc_freq_mhz: 0,
        vendor: CpuVendor::Unknown,
        features: CpuFeatures::empty(),
        topology: CpuTopology {
            physical_cores: 1,
            logical_cores: 1,
            apic_id: 0,
            smt: false,
        },
        stepping: 0,
        model: 0,
        family: 0,
    };

    unsafe {
        // === Leaf 0x00: vendor + max leaf ===
        let (max_leaf, ebx, ecx, edx) = cpuid_raw(0, 0);
        info.max_leaf = max_leaf;
        info.vendor = detect_vendor(ebx, ecx, edx);

        // === Leaf 0x01: family/model/stepping + feature flags ===
        if max_leaf >= 1 {
            let (eax, ebx, ecx, edx) = cpuid_raw(1, 0);
            info.cores = ((ebx >> 16) & 0xFF) as u32;
            if info.cores == 0 { info.cores = 1; }

            let (family, model, stepping) = detect_family_model_stepping(eax);
            info.family = family;
            info.model = model;
            info.stepping = stepping;

            // Topology
            info.topology.apic_id = (ebx >> 24) & 0xFF;
            info.topology.logical_cores = info.cores;

            // Feature flags — EDX
            let f = &mut info.features;
            f.fpu = edx & (1 << 0) != 0;
            f.fxsave_fxstor = edx & (1 << 24) != 0;
            f.sse = edx & (1 << 25) != 0;
            f.sse2 = edx & (1 << 26) != 0;
            f.tsc = edx & (1 << 4) != 0;
            f.msr = edx & (1 << 5) != 0;
            f.apic = edx & (1 << 9) != 0;
            f.cx8 = edx & (1 << 8) != 0;
            f.nx = edx & (1 << 20) != 0;
            f.pse = edx & (1 << 3) != 0;
            f.pge = edx & (1 << 11) != 0;
            f.pat = edx & (1 << 16) != 0;
            f.acpi = edx & (1 << 22) != 0;
            f.mmx = edx & (1 << 23) != 0;

            // Feature flags — ECX
            f.sse3 = ecx & (1 << 0) != 0;
            f.ssse3 = ecx & (1 << 9) != 0;
            f.sse4_1 = ecx & (1 << 19) != 0;
            f.sse4_2 = ecx & (1 << 20) != 0;
            f.avx = ecx & (1 << 28) != 0;
            f.f16c = ecx & (1 << 29) != 0;
            f.popcnt = ecx & (1 << 23) != 0;
            f.xsave = ecx & (1 << 26) != 0;
            f.osxsave = ecx & (1 << 27) != 0;
        }

        // === Leaf 0x07: extended features ===
        if max_leaf >= 7 {
            let (_, ebx, _, _) = cpuid_raw(7, 0);
            let f = &mut info.features;
            f.avx2 = ebx & (1 << 5) != 0;
            f.bmi1 = ebx & (1 << 3) != 0;
            f.bmi2 = ebx & (1 << 8) != 0;
            f.smep = ebx & (1 << 7) != 0;
            f.smap = ebx & (1 << 20) != 0;
        }

        // === Extended leaves ===
        let (max_ext_leaf, _, _, _) = cpuid_raw(0x80000000, 0);
        info.max_ext_leaf = max_ext_leaf;

        // Leaf 0x80000001: long mode, syscall, 1GB pages
        if max_ext_leaf >= 0x80000001 {
            let (_, _, ecx, edx) = cpuid_raw(0x80000001, 0);
            info.features.lm = edx & (1 << 29) != 0;
            info.features.syscall_sysret = edx & (1 << 11) != 0;
            info.features.pdpe1gb = ecx & (1 << 26) != 0;
        }

        // Brand string (leaves 0x80000002..0x80000004)
        if max_ext_leaf >= 0x80000004 {
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
            info.brand_len = info.brand.iter().position(|&b| b == 0).unwrap_or(48);
        }

        // === Leaf 0x0B: topology (Intel) ===
        if max_leaf >= 0x0B {
            let (eax, ebx, _, _) = cpuid_raw(0x0B, 0);
            let level_cores = (ebx & 0xFF) as u32;
            let level_type = (eax >> 8) & 0xF;
            if level_type == 1 { // SMT level
                info.topology.smt = level_cores > 1;
            }
            // Sub-leaf 1 for core count
            let (_, ebx2, _, _) = cpuid_raw(0x0B, 1);
            let core_apic_id = (ebx2 & 0xFF) as u32;
            info.topology.apic_id = core_apic_id;
            let logical_at_level = (ebx2 & 0xFFFF) as u32;
            if logical_at_level > 0 {
                info.topology.logical_cores = logical_at_level;
            }
        }

        // TSC frequency через platform info MSR (0xCE)
        let platform_info = rdmsr(0xCE);
        let ratio = (platform_info >> 8) & 0xFF;
        let bus_freq = (platform_info >> 16) & 0xFF;
        if ratio > 0 && bus_freq > 0 {
            info.tsc_freq_mhz = (ratio as u64) * (bus_freq as u64);
        } else {
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
        "QEMU Virtual CPU"
    } else {
        s
    }
}

/// Вернуть строку vendor
pub fn vendor_str(info: &CpuInfo) -> &str {
    match info.vendor {
        CpuVendor::Intel => "Intel",
        CpuVendor::Amd => "AMD",
        CpuVendor::Unknown => "Unknown",
    }
}

/// Проверить наличие feature flag
pub fn has_feature(features: &CpuFeatures, flag: FeatureFlag) -> bool {
    match flag {
        FeatureFlag::SSE => features.sse,
        FeatureFlag::SSE2 => features.sse2,
        FeatureFlag::SSE3 => features.sse3,
        FeatureFlag::SSSE3 => features.ssse3,
        FeatureFlag::SSE4_1 => features.sse4_1,
        FeatureFlag::SSE4_2 => features.sse4_2,
        FeatureFlag::AVX => features.avx,
        FeatureFlag::AVX2 => features.avx2,
        FeatureFlag::NX => features.nx,
        FeatureFlag::APIC => features.apic,
        FeatureFlag::TSC => features.tsc,
        FeatureFlag::SMEP => features.smep,
        FeatureFlag::SMAP => features.smap,
        FeatureFlag::FPU => features.fpu,
        FeatureFlag::MMX => features.mmx,
        FeatureFlag::LONG_MODE => features.lm,
        FeatureFlag::PDPE1GB => features.pdpe1gb,
        FeatureFlag::SYSCALL => features.syscall_sysret,
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FeatureFlag {
    SSE, SSE2, SSE3, SSSE3, SSE4_1, SSE4_2,
    AVX, AVX2,
    NX, APIC, TSC, FPU, MMX,
    SMEP, SMAP,
    LONG_MODE, PDPE1GB, SYSCALL,
}
