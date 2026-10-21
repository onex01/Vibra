#![no_std]
#![no_main]

use vibra_kernel as kernel;

/// Временная x86_64-точка входа kernel-only сборки.
/// Пользовательская оболочка и GUI не входят в этот бинарник.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    kernel::boot()
}
