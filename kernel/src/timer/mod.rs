// Timer Subsystem — единый интерфейс для аппаратных таймеров ядра.
//
// Стратегия: HPET → PIT (fallback).
//   1. HPET (High Precision Event Timer): MMIO, обычно 10-24 МГц
//   2. PIT (Programmable Interval Timer): 1.193182 МГц (legacy fallback)
//
// API:
//   init()             — попытка HPET, fallback PIT 100 Hz
//   current_ticks()    — текущее значение аппаратного счётчика
//   ticks_per_second() — частота таймера (тактов в секунду)
//   sleep_ms(ms)       — busy-wait на N миллисекунд

pub mod hpet;

use crate::println;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// ======================== Timer Backend ========================

#[derive(Debug, Clone, Copy, PartialEq)]
enum TimerBackend {
    Hpet,
    Pit,
}

static CURRENT_BACKEND: AtomicU64 = AtomicU64::new(0); // 0 = None, 1 = Hpet, 2 = Pit
static TICKS_PER_SEC: AtomicU64 = AtomicU64::new(1_193_182); // PIT default

// ======================== PIT Fallback ========================

/// PIT base port + divisor for 100 Hz
const PIT_FREQ: u64 = 1_193_182;
const PIT_HZ: u64 = 100;

/// PIT channel 0 counter port
const PIT_CH0_DATA: u16 = 0x40;

unsafe fn pit_sleep_ms(ms: u64) {
    // PIT channel 0 tick rate = 1193182 Hz, т.е. 1 тик ≈ 838 нс
    // Busy-wait через чтение PIT channel 0 (one-shot countdown)
    // Используем PIT channel 2 для точного timing (как в apic.rs)

    let pit_channel2: u16 = 0x42;
    let pit_cmd: u16 = 0x43;
    let pit_sc: u16 = 0x61;

    let target_us = ms * 1000;
    let pit_count = (PIT_FREQ * target_us / 1_000_000) as u16;

    // PIT Channel 2: one-shot mode (mode 0), lobyte/hibyte
    core::arch::asm!("out dx, al", in("dx") pit_cmd, in("al") 0xB0u8, options(nomem, nostack, preserves_flags));
    core::arch::asm!("out dx, al", in("dx") pit_channel2, in("al") (pit_count & 0xFF) as u8, options(nomem, nostack, preserves_flags));
    core::arch::asm!("out dx, al", in("dx") pit_channel2, in("al") (pit_count >> 8) as u8, options(nomem, nostack, preserves_flags));

    // Включаем PIT channel 2 gate
    let sc: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") sc, in("dx") pit_sc, options(nomem, nostack, preserves_flags));
        core::arch::asm!("out dx, al", in("dx") pit_sc, in("al") sc | 1u8, options(nomem, nostack, preserves_flags));
    }

    // Ожидаем TMR2_OUT (bit 5 of port 0x61)
    let mut timeout: u32 = 10_000_000;
    unsafe {
        loop {
            let val: u8;
            core::arch::asm!("in al, dx", out("al") val, in("dx") pit_sc, options(nomem, nostack, preserves_flags));
            if val & 0x20 != 0 {
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                break;
            }
        }
    }
}

// ======================== Public API ========================

/// Инициализация таймерной подсистемы.
/// Пытается HPET, fallback PIT.
pub fn init() {
    println!("[TIMER] Инициализация таймерной подсистемы...");

    // Получаем HPET base из ACPI если есть
    let acpi_hpet = crate::acpi::get()
        .as_ref()
        .and_then(|a| a.hpet_base);

    // Попытка HPET
    if hpet::init(acpi_hpet) {
        CURRENT_BACKEND.store(1, Ordering::SeqCst);
        TICKS_PER_SEC.store(hpet::ticks_per_second(), Ordering::SeqCst);
        println!("[TIMER] Бэкенд: HPET ({} Гц)", hpet::ticks_per_second());
    } else {
        // Fallback: PIT channel 0 уже запущен в idt.rs (100 Hz)
        CURRENT_BACKEND.store(2, Ordering::SeqCst);
        TICKS_PER_SEC.store(PIT_FREQ, Ordering::SeqCst);
        println!("[TIMER] Бэкенд: PIT fallback ({} Гц)", PIT_HZ);
    }

    println!("[TIMER] Инициализация завершена");
}

/// Текущее значение аппаратного счётчика
pub fn current_ticks() -> u64 {
    let backend = CURRENT_BACKEND.load(Ordering::Relaxed);
    match backend {
        1 => hpet::read_counter(),
        2 => {
            // PIT: читаем channel 0 current counter
            // PIT не имеет простого чтения текущего счётчика из user space,
            // используем ticks из IDT
            crate::interrupts::idt::TICKS.load(Ordering::Relaxed)
        }
        _ => 0,
    }
}

/// Частота таймера (тактов в секунду)
pub fn ticks_per_second() -> u64 {
    TICKS_PER_SEC.load(Ordering::Relaxed)
}

/// Busy-wait на N миллисекунд
pub fn sleep_ms(ms: u64) {
    let backend = CURRENT_BACKEND.load(Ordering::Relaxed);
    match backend {
        1 => hpet::sleep_ms(ms),
        2 => unsafe { pit_sleep_ms(ms); },
        _ => {
            // Нет таймера — простой busy loop
            let ticks_per_ms = 1_000_000 / 1000; // ~1000 iterations per ms
            for _ in 0..ms * ticks_per_ms {
                core::hint::spin_loop();
            }
        }
    }
}
