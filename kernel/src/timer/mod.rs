// Timer / clock subsystem.
//
// Источники монотонного времени, в порядке предпочтения:
//
//   1. TSC (если CPUID сообщает invariant TSC) — разрешение ~1 нс,
//      без MMIO-чтений, калибруется по HPET (или по PIT channel 2).
//   2. HPET 64-bit main counter — частота берётся из ID-регистра.
//   3. PIT IRQ0 (100 Гц) — самый последний вариант: тикает только при
//      доставке прерываний.
//
// IRQ-тики (`interrupts::idt::TICKS`) нужны ТОЛЬКО планировщику. Время
// (uptime, wall clock, тайм-ауты драйверов) считается по `now_ns()` и не
// зависит от того, доставляется ли IRQ0 на конкретной машине. Именно
// поэтому на реальном ноутбуке, где PIC IRQ0 молчит, `date`/`uptime` раньше
// показывали нули, а shell мог навсегда остаться в `hlt`.

pub mod hpet;
pub mod rtc;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::interrupts::idt::TIMER_HZ;

const NS_PER_SEC: u64 = 1_000_000_000;

/// PIT: 1.193182 МГц, канал 2 используется только для калибровки.
const PIT_FREQ: u64 = 1_193_182;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockSource {
    None,
    Tsc,
    Hpet,
    PitIrq,
}

impl ClockSource {
    pub fn name(self) -> &'static str {
        match self {
            ClockSource::None => "none",
            ClockSource::Tsc => "TSC",
            ClockSource::Hpet => "HPET",
            ClockSource::PitIrq => "PIT IRQ",
        }
    }
}

static SOURCE: AtomicU64 = AtomicU64::new(0); // 0 none, 1 tsc, 2 hpet, 3 pit-irq
static SOURCE_HZ: AtomicU64 = AtomicU64::new(0);
/// Множитель «наносекунд на тик» в формате Q32 (фиксированная точка 32.32).
static NS_PER_TICK_Q32: AtomicU64 = AtomicU64::new(0);
/// Значение счётчика на момент старта (uptime считается от него).
static BASE_RAW: AtomicU64 = AtomicU64::new(0);
/// TSC откалиброван и invariant.
static TSC_OK: AtomicBool = AtomicBool::new(false);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// IRQ-тики реально приходят (PIC IRQ0 или LAPIC fallback).
static IRQ_TICKS_HEALTHY: AtomicBool = AtomicBool::new(false);

// ======================== Низкий уровень ========================

#[inline]
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value,
        options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port,
        options(nostack, preserves_flags));
    value
}

/// Множитель Q32 для перевода тиков частоты `hz` в наносекунды.
fn q32_ns_per_tick(hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    (((NS_PER_SEC as u128) << 32) / hz as u128) as u64
}

#[inline]
fn raw_to_ns(raw: u64) -> u64 {
    let mult = NS_PER_TICK_Q32.load(Ordering::Relaxed);
    let delta = raw.wrapping_sub(BASE_RAW.load(Ordering::Relaxed));
    (((delta as u128) * (mult as u128)) >> 32) as u64
}

#[inline]
fn source() -> ClockSource {
    match SOURCE.load(Ordering::Relaxed) {
        1 => ClockSource::Tsc,
        2 => ClockSource::Hpet,
        3 => ClockSource::PitIrq,
        _ => ClockSource::None,
    }
}

fn set_source(src: ClockSource, hz: u64, base_raw: u64) {
    let tag = match src {
        ClockSource::None => 0,
        ClockSource::Tsc => 1,
        ClockSource::Hpet => 2,
        ClockSource::PitIrq => 3,
    };
    SOURCE_HZ.store(hz, Ordering::SeqCst);
    NS_PER_TICK_Q32.store(q32_ns_per_tick(hz), Ordering::SeqCst);
    BASE_RAW.store(base_raw, Ordering::SeqCst);
    SOURCE.store(tag, Ordering::SeqCst);
}

/// Проверить invariant TSC (CPUID.80000007H:EDX[8]).
fn has_invariant_tsc() -> bool {
    unsafe {
        let max_ext = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        if max_ext < 0x8000_0007 {
            return false;
        }
        core::arch::x86_64::__cpuid(0x8000_0007).edx & (1 << 8) != 0
    }
}

/// Калибровка TSC по HPET: окно 20 мс.
fn calibrate_tsc_hpet() -> Option<u64> {
    if !hpet::is_ready() {
        return None;
    }
    let hz = hpet::counter_hz();
    let window = hz / 50; // 20 мс
    let h0 = hpet::read_counter();
    let t0 = rdtsc();
    while hpet::read_counter().wrapping_sub(h0) < window {
        core::hint::spin_loop();
    }
    let t1 = rdtsc();
    let h1 = hpet::read_counter();

    let dh = h1.wrapping_sub(h0);
    let dt = t1.wrapping_sub(t0);
    if dh == 0 || dt == 0 {
        return None;
    }
    let tsc_hz = ((dt as u128 * hz as u128) / dh as u128) as u64;
    if (100_000_000..=20_000_000_000).contains(&tsc_hz) {
        Some(tsc_hz)
    } else {
        None
    }
}

/// Калибровка TSC по PIT channel 2 (10 мс), если HPET недоступен.
fn calibrate_tsc_pit() -> Option<u64> {
    let count = (PIT_FREQ / 100) as u16; // 10 мс

    unsafe {
        // Канал 2, mode 0 (one-shot), lobyte/hibyte.
        outb(0x43, 0xB0);
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);

        // Gate 2 = 0 (стоп), спикер выключен.
        let mut sc = inb(0x61);
        sc &= !0x02;
        sc &= !0x01;
        outb(0x61, sc);

        // Gate 2 = 1 — счётчик стартует, OUT уходит в 0.
        sc |= 0x01;
        outb(0x61, sc);
    }

    let t0 = rdtsc();
    let mut expired = false;
    for _ in 0..200_000_000u32 {
        if unsafe { inb(0x61) } & 0x20 != 0 {
            expired = true;
            break;
        }
        core::hint::spin_loop();
    }
    let t1 = rdtsc();
    if !expired {
        return None;
    }

    let dt = t1.wrapping_sub(t0);
    if dt == 0 {
        return None;
    }
    // 10 мс → секунда = ×100.
    let tsc_hz = dt.saturating_mul(100);
    if (100_000_000..=20_000_000_000).contains(&tsc_hz) {
        Some(tsc_hz)
    } else {
        None
    }
}

// ======================== Инициализация ========================

/// Инициализация часов. Вызывается один раз на шаге `[BOOT] Step 8`.
pub fn init() {
    crate::println!("[TIMER] Probing clocks...");

    let acpi_hpet = crate::acpi::get().as_ref().and_then(|a| a.hpet_base);
    let hpet_ok = hpet::init(acpi_hpet);

    let mut tsc_hz = None;
    if has_invariant_tsc() {
        tsc_hz = calibrate_tsc_hpet().or_else(calibrate_tsc_pit);
        match tsc_hz {
            Some(hz) => {
                TSC_HZ.store(hz, Ordering::SeqCst);
                TSC_OK.store(true, Ordering::SeqCst);
                crate::println!(
                    "[TIMER] TSC invariant, calibrated: {} Hz ({} MHz)",
                    hz,
                    hz / 1_000_000
                );
            }
            None => crate::println!("[TIMER] TSC calibration failed"),
        }
    } else {
        crate::println!("[TIMER] TSC is not invariant; not used as a clock");
    }

    match (tsc_hz, hpet_ok) {
        (Some(hz), _) => {
            set_source(ClockSource::Tsc, hz, rdtsc());
            crate::println!("[TIMER] Clock source: TSC ({} Hz)", hz);
        }
        (None, true) => {
            let hz = hpet::counter_hz();
            set_source(ClockSource::Hpet, hz, hpet::read_counter());
            crate::println!("[TIMER] Clock source: HPET ({} Hz)", hz);
        }
        (None, false) => {
            set_source(ClockSource::PitIrq, TIMER_HZ, 0);
            crate::println!(
                "[TIMER] Clock source: PIT IRQ ticks ({} Hz) — no hardware counter",
                TIMER_HZ
            );
        }
    }

    // Время BIOS → база wall clock.
    match rtc::read() {
        Some(t) => {
            let unix = t.to_unix();
            let now = now_ns();
            rtc::set_boot_time(unix, now);
            crate::println!(
                "[TIMER] RTC (BIOS): {:04}-{:02}-{:02} {:02}:{:02}:{:02} (unix {})",
                t.year, t.month, t.day, t.hour, t.minute, t.second, unix
            );
        }
        None => crate::println!("[TIMER] RTC unavailable — wall clock disabled"),
    }

    crate::println!("[TIMER] Initialization complete");
}


// ======================== Публичный API ========================

/// Текущее монотонное время в наносекундах с момента старта часов.
///
/// Не зависит от доставки прерываний: TSC/HPET читаются напрямую.
pub fn now_ns() -> u64 {
    match source() {
        ClockSource::Tsc => raw_to_ns(rdtsc()),
        ClockSource::Hpet => raw_to_ns(hpet::read_counter()),
        ClockSource::PitIrq => {
            crate::interrupts::idt::ticks().saturating_mul(NS_PER_SEC / TIMER_HZ)
        }
        ClockSource::None => 0,
    }
}

/// Монотонное время с момента старта, наносекунды.
pub fn uptime_ns() -> u64 {
    now_ns()
}

/// Монотонное время с момента старта, микросекунды.
pub fn uptime_us() -> u64 {
    now_ns() / 1_000
}

/// Монотонное время с момента старта, миллисекунды.
pub fn uptime_ms() -> u64 {
    now_ns() / 1_000_000
}

/// Частота выбранного источника (Гц). 0 — источник не выбран.
pub fn source_hz() -> u64 {
    SOURCE_HZ.load(Ordering::Relaxed)
}

pub fn current_source() -> ClockSource {
    source()
}

/// Откалиброванная частота TSC (если TSC используется/доступен).
pub fn tsc_hz() -> Option<u64> {
    if TSC_OK.load(Ordering::Relaxed) {
        Some(TSC_HZ.load(Ordering::Relaxed))
    } else {
        None
    }
}

/// Busy-wait. Не полагается на прерывания — работает и до `interrupts::enable()`.
pub fn sleep_ms(ms: u64) {
    if ms == 0 {
        return;
    }
    let target = ms.saturating_mul(1_000_000);
    match source() {
        ClockSource::None => {
            // Часы ещё не инициализированы (ранний boot): грубая задержка.
            for _ in 0..ms.saturating_mul(100_000) {
                core::hint::spin_loop();
            }
        }
        _ => {
            let start = now_ns();
            let mut last = start;
            let mut stalled = 0u32;
            loop {
                let now = now_ns();
                if now.wrapping_sub(start) >= target {
                    break;
                }
                // Источник (PIT IRQ) может не тикать при выключенных
                // прерываниях — не зависаем навсегда.
                if now == last {
                    stalled += 1;
                    if stalled > 50_000_000 {
                        break;
                    }
                } else {
                    stalled = 0;
                    last = now;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Частота IRQ-тиков, измеренная по монотонным часам.
pub fn measure_irq_hz(window_ms: u64) -> u64 {
    let window = window_ms.max(1);
    let t0 = crate::interrupts::idt::ticks();
    let m0 = now_ns();
    sleep_ms(window);
    let dt = crate::interrupts::idt::ticks().wrapping_sub(t0);
    let dm = now_ns().saturating_sub(m0);
    if dm == 0 {
        return 0;
    }
    ((dt as u128 * NS_PER_SEC as u128) / dm as u128) as u64
}

/// Проверка источника тиков после `interrupts::enable()`.
///
/// Если PIC IRQ0 не доставляется вообще (частая ситуация на ноутбуках, где
/// таймер уходит в IOAPIC, а PIC не подключён), поднимаем LAPIC timer.
/// Если и это невозможно — помечаем IRQ-тики нездоровыми, и shell будет
/// опрашивать ввод busy-loop вместо `hlt`.
pub fn post_init() {
    if source() == ClockSource::None {
        return;
    }

    let t0 = crate::interrupts::idt::ticks();
    let m0 = now_ns();
    // Окно 200 мс: при 100 Гц это ~20 тиков, квантование ±5%
    // (окно 50 мс давало всего 5-6 тиков и «плавающие» 110-120 Гц).
    sleep_ms(200);
    let dt = crate::interrupts::idt::ticks().wrapping_sub(t0);
    let dm = now_ns().saturating_sub(m0);
    let measured = if dm > 0 {
        ((dt as u128 * NS_PER_SEC as u128) / dm as u128) as u64
    } else {
        0
    };

    if dt == 0 {
        if crate::interrupts::apic::start_periodic_timer(TIMER_HZ as u32) {
            IRQ_TICKS_HEALTHY.store(true, Ordering::SeqCst);
            // PIC IRQ0 молчит — маскируем, чтобы не получить двойные тики,
            // если прошивка всё-таки начнёт их доставлять.
            unsafe { crate::interrupts::pic::mask_irq0(); }
            crate::println!(
                "[TIMER] PIC IRQ0 silent → LAPIC timer fallback at {} Hz",
                TIMER_HZ
            );
        } else {
            IRQ_TICKS_HEALTHY.store(false, Ordering::SeqCst);
            crate::println!(
                "[TIMER] WARNING: no IRQ tick source; uptime uses hardware clock, idle busy-polls"
            );
        }
        return;
    }

    IRQ_TICKS_HEALTHY.store(true, Ordering::SeqCst);
    crate::println!(
        "[TIMER] IRQ ticks: {} in 200 ms → {} Hz (expected {})",
        dt,
        measured,
        TIMER_HZ
    );

    let lo = TIMER_HZ - TIMER_HZ / 6;
    let hi = TIMER_HZ + TIMER_HZ / 6;
    if measured < lo || measured > hi {
        crate::println!(
            "[TIMER] WARNING: measured IRQ rate {} Hz differs from {} Hz; time is taken from the hardware clock, so uptime stays correct",
            measured,
            TIMER_HZ
        );
    }
}

/// Приходят ли IRQ-тики (PIC IRQ0 или LAPIC fallback).
pub fn irq_ticks_healthy() -> bool {
    IRQ_TICKS_HEALTHY.load(Ordering::Relaxed)
}

/// Ожидание следующего прерывания.
///
/// `hlt` возвращается только по прерыванию, поэтому если тикового источника
/// нет, `hlt` может усыпить машину навсегда (клавиатура при polling fallback
/// прерываний не генерирует). В этом случае крутимся в `spin_loop`.
pub fn idle_wait() {
    if irq_ticks_healthy() {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
    } else {
        core::hint::spin_loop();
    }
}

/// Текущее время BIOS + монотонных часов.
pub fn wall_clock() -> Option<rtc::DateTime> {
    rtc::wall_clock()
}

/// Сводка для `diag timer`.
#[derive(Clone, Copy)]
pub struct ClockInfo {
    pub source: ClockSource,
    pub source_hz: u64,
    pub uptime_ns: u64,
    pub irq_ticks: u64,
    pub irq_healthy: bool,
    pub tsc_hz: Option<u64>,
    pub hpet_ready: bool,
    pub hpet_base: u64,
    pub hpet_period_fs: u64,
    pub hpet_hz: u64,
    pub wall_clock: Option<rtc::DateTime>,
    pub boot_unix: u64,
    pub rtc: Option<rtc::RtcTime>,
}

pub fn clock_info() -> ClockInfo {
    ClockInfo {
        source: source(),
        source_hz: source_hz(),
        uptime_ns: now_ns(),
        irq_ticks: crate::interrupts::idt::ticks(),
        irq_healthy: irq_ticks_healthy(),
        tsc_hz: tsc_hz(),
        hpet_ready: hpet::is_ready(),
        hpet_base: hpet::physical_address(),
        hpet_period_fs: hpet::period_fs(),
        hpet_hz: hpet::counter_hz(),
        wall_clock: rtc::wall_clock(),
        boot_unix: rtc::boot_unix(),
        rtc: rtc::read(),
    }
}

