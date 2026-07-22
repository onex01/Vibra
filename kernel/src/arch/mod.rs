// Arch module — определение архитектуры и re-exports.
//
// Архитектура определяется target triple:
// - x86_64-unknown-none → x86 модули
// - aarch64-unknown-none → arm64 модули

pub mod x86;
pub mod arm64;

/// Имя текущей архитектуры
pub fn arch_name() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    { "x86_64" }
    #[cfg(target_arch = "aarch64")]
    { "aarch64" }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { "unknown" }
}

/// Инициализация arch-specific модулей
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86::init();
}

/// Имя загрузчика
pub fn boot_loader() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    { "Limine (UEFI)" }
    #[cfg(target_arch = "aarch64")]
    { "UEFI" }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    { "Unknown" }
}
