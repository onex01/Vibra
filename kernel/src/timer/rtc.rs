// RTC (CMOS) — часы реального времени / BIOS-время.
//
// На x86 время BIOS доступно через индексный порт 0x70 и порт данных 0x71.
// До сих пор ядро вообще не читало RTC: команда `date` печатала uptime,
// поэтому «время не синхронизировалось с ядром». Здесь реализовано:
//
//   * ожидание сброса флага UIP (Update In Progress) перед чтением;
//   * двойное чтение всех регистров до совпадения (защита от гонки с
//     обновлением счётчиков, которое происходит раз в секунду);
//   * декодирование BCD (Status Register B, бит 2);
//   * 12/24-часовой режим (Status Register B, бит 1 + бит PM в часе);
//   * век (century) из поля CENTURY таблицы FADT, если прошивка его задала;
//   * перевод в Unix-время (алгоритм days_from_civil Говарда Хиннанта).

use core::sync::atomic::{AtomicU64, Ordering};

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const REG_SECONDS: u8 = 0x00;
const REG_MINUTES: u8 = 0x02;
const REG_HOURS: u8 = 0x04;
const REG_DAY: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
const REG_STATUS_A: u8 = 0x0A;
const REG_STATUS_B: u8 = 0x0B;

/// Некуда не денемся: если FADT не сообщил регистр века, используем типовой.
const DEFAULT_CENTURY_REG: u8 = 0x32;

const UIP: u8 = 0x80; // Status Register A: Update In Progress
const BCD_MODE: u8 = 0x04; // Status Register B: 0 = BCD, 1 = binary
const HOUR_24: u8 = 0x02; // Status Register B: 1 = 24-hour mode

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

/// Прочитать регистр CMOS. Бит 7 адреса отключает NMI на время доступа —
/// так делает и BIOS, иначе NMI может прийти между индексом и данными.
fn cmos_read(reg: u8) -> u8 {
    unsafe {
        outb(CMOS_ADDR, 0x80 | (reg & 0x7F));
        inb(CMOS_DATA)
    }
}

fn bcd_to_bin(v: u8) -> u8 {
    (v & 0x0F) + ((v >> 4) & 0x0F) * 10
}

/// Сырые значения регистров (до декодирования) — полезно для диагностики.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawRtc {
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8,
    pub century: u8,
    pub status_b: u8,
}

impl RawRtc {
    /// Значение регистра века, если прошивка его заполняет (иначе 0).
    pub fn century_value(&self) -> Option<u16> {
        let c = bcd_to_bin(self.century);
        if (19..=21).contains(&c) { Some(c as u16) } else { None }
    }
}

/// Расшифрованное время BIOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl RtcTime {
    /// Unix-время (секунды с 1970-01-01). RTC трактуется как UTC:
    /// часовой пояс прошивка не сообщает, поэтому сдвиг не применяется.
    pub fn to_unix(&self) -> u64 {
        let days = days_from_civil(self.year as i64, self.month as i64, self.day as i64);
        let secs = days * 86_400
            + self.hour as i64 * 3_600
            + self.minute as i64 * 60
            + self.second as i64;
        if secs < 0 { 0 } else { secs as u64 }
    }

    pub fn is_valid(&self) -> bool {
        (1970..=2199).contains(&self.year)
            && (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }
}

/// Считать регистры RTC. Возвращает `None`, если CMOS не отвечает.
pub fn read_raw() -> Option<RawRtc> {
    // Ждём, пока завершится текущий цикл обновления (UIP = 0).
    let mut waited = false;
    for _ in 0..100_000u32 {
        if cmos_read(REG_STATUS_A) & UIP == 0 {
            waited = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !waited {
        return None;
    }

    let century_reg = century_register();

    // Двойное чтение: если между ними прошло обновление, значения
    // разойдутся — тогда повторяем.
    for _ in 0..5 {
        let status_b = cmos_read(REG_STATUS_B);
        let mut a = RawRtc { status_b, ..Default::default() };
        let mut b = RawRtc { status_b, ..Default::default() };
        let regs = [
            REG_SECONDS, REG_MINUTES, REG_HOURS, REG_DAY,
            REG_MONTH, REG_YEAR, century_reg,
        ];
        for (i, &reg) in regs.iter().enumerate() {
            let v = cmos_read(reg);
            match i {
                0 => a.second = v,
                1 => a.minute = v,
                2 => a.hour = v,
                3 => a.day = v,
                4 => a.month = v,
                5 => a.year = v,
                _ => a.century = v,
            }
        }
        for (i, &reg) in regs.iter().enumerate() {
            let v = cmos_read(reg);
            match i {
                0 => b.second = v,
                1 => b.minute = v,
                2 => b.hour = v,
                3 => b.day = v,
                4 => b.month = v,
                5 => b.year = v,
                _ => b.century = v,
            }
        }
        if a == b {
            return Some(a);
        }
    }

    None
}

/// Декодировать сырые регистры в календарное время.
pub fn decode(raw: &RawRtc) -> Option<RtcTime> {
    let bcd = raw.status_b & BCD_MODE == 0;
    let dec = |v: u8| if bcd { bcd_to_bin(v) } else { v };

    let second = dec(raw.second);
    let minute = dec(raw.minute);

    let mut hour = raw.hour;
    if raw.status_b & HOUR_24 == 0 {
        // 12-часовой режим: бит 7 — PM.
        let pm = hour & 0x80 != 0;
        hour = dec(hour & 0x7F);
        if hour == 12 {
            hour = 0;
        }
        if pm {
            hour += 12;
        }
    } else {
        hour = dec(hour);
    }

    let day = dec(raw.day);
    let month = dec(raw.month);
    let year2 = dec(raw.year);

    let century = raw.century_value().unwrap_or_else(|| {
        // Эвристика: 70..99 → 19xx, 00..69 → 20xx.
        if year2 >= 70 { 19 } else { 20 }
    });
    let year = century * 100 + year2 as u16;

    // Регистры могут содержать мусор (например, 0xFF), если CMOS не отвечает.
    let time = RtcTime { year, month, day, hour, minute, second };
    if time.is_valid() { Some(time) } else { None }
}

/// Прочитать и сразу декодировать время BIOS.
pub fn read() -> Option<RtcTime> {
    let raw = read_raw()?;
    decode(&raw)
}

/// Регистр века: FADT.CENTURY, иначе типовой 0x32.
fn century_register() -> u8 {
    crate::acpi::get()
        .as_ref()
        .and_then(|info| info.rtc_century_index)
        .filter(|idx| *idx != 0 && *idx < 0x80)
        .unwrap_or(DEFAULT_CENTURY_REG)
}

// ======================== Календарная арифметика ========================

/// Дни с 1970-01-01 для гражданской даты (Howard Hinnant, days_from_civil).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // 0..399
    let mp = (m + 9) % 12; // 0..11 (март = 0)
    let doy = (153 * mp + 2) / 5 + d - 1; // 0..365
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Обратная операция: (год, месяц, день) из числа дней с 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // 0..146096
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// Дата/время, полученные из Unix-времени.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub unix: u64,
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl DateTime {
    pub fn from_unix(unix: u64) -> Self {
        let days = (unix / 86_400) as i64;
        let rem = unix % 86_400;
        let (year, month, day) = civil_from_days(days);
        Self {
            unix,
            year,
            month,
            day,
            hour: (rem / 3_600) as u32,
            minute: ((rem % 3_600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }

    /// `YYYY-MM-DD HH:MM:SS`
    pub fn format(&self) -> alloc::string::String {
        alloc::format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day,
            self.hour, self.minute, self.second
        )
    }
}

// ======================== Wall clock ========================
//
// Ядро хранит только пару (время BIOS на момент старта, монотонные
// наносекунды на момент старта). Дальше время = RTC + (now_ns - boot_ns),
// поэтому оно не зависит от частоты IRQ и не «замирает» на реальном железе.

static BOOT_UNIX: AtomicU64 = AtomicU64::new(0);
static BOOT_NS: AtomicU64 = AtomicU64::new(0);

/// Зафиксировать стартовое время BIOS. Вызывается один раз из `timer::init`.
pub fn set_boot_time(unix: u64, boot_ns: u64) {
    BOOT_UNIX.store(unix, Ordering::SeqCst);
    BOOT_NS.store(boot_ns, Ordering::SeqCst);
}

pub fn has_wall_clock() -> bool {
    BOOT_UNIX.load(Ordering::Relaxed) != 0
}

pub fn boot_unix() -> u64 {
    BOOT_UNIX.load(Ordering::Relaxed)
}

/// Текущее время по RTC-базе и монотонным часам.
pub fn wall_clock() -> Option<DateTime> {
    let base = BOOT_UNIX.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let boot_ns = BOOT_NS.load(Ordering::Relaxed);
    let now = crate::timer::uptime_ns();
    let elapsed_s = now.saturating_sub(boot_ns) / 1_000_000_000;
    Some(DateTime::from_unix(base + elapsed_s))
}

