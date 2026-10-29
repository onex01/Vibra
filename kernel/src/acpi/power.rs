// Выключение питания через ACPI.
//
// # Почему это отдельный модуль
//
// Раньше `quit` писал `0x2000` в порт `0x604` и `0x03` в порт `0x92`.
// Оба варианта неверны для реального железа:
//
// - `0x604` — это PM1a_CNT **только у QEMU q35**. На настоящей машине по
//   этому адресу ничего нет, запись уходит в пустоту;
// - `0x92` — регистр ACPI reset: бит 0 перезагружает машину, бит 1 управляет
//   A20. То есть на железе `quit` либо ничего не делал, либо перезагружал.
//
// Правильный путь: взять из FADT адрес блока PM1a_CNT и записать туда
// `SLP_TYP | SLP_EN`. Значение SLP_TYP для состояния S5 не является
// константой — оно объявлено в AML-пакете `_S5` внутри DSDT. Разбирать весь
// AML не нужно: `_S5` — статический пакет из двух констант, достаточно найти
// его в таблице байтов.

use super::sdt::SdtHeader;

/// Бит включения перехода в сон в PM1_CNT.
const SLP_EN: u16 = 1 << 13;
/// Бит SCI_EN: ACPI управляет питанием, а не legacy SMI.
const SCI_EN: u16 = 1 << 0;

/// Значение SLP_TYP по умолчанию, если `_S5` разобрать не удалось.
///
/// Пятёрка — то, что объявляют почти все платформы. Используется только как
/// последняя попытка: если `_S5` найден, берётся именно его значение.
const DEFAULT_S5_SLP_TYP: u16 = 5;

unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", out("ax") value, in("dx") port, options(nostack, preserves_flags));
    value
}

unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nostack, preserves_flags));
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nostack, preserves_flags));
    value
}

unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nostack, preserves_flags));
}

fn io_delay() {
    for _ in 0..2_000_000u32 {
        core::hint::spin_loop();
    }
}

/// Перезагрузка машины.
///
/// Порядок:
/// 1. FADT RESET_REG/RESET_VALUE (самый корректный путь по ACPI);
/// 2. chipset reset через 0xCF9 (0x02 → 0x06);
/// 3. ACPI reset 0x92, бит 0;
/// 4. i8042 0x64/0xFE.
///
/// Если ни один не сработал, вызывающий делает triple fault — это лучше, чем
/// навсегда остаться в `hlt` на реальной машине.
pub fn reset() -> Result<(), &'static str> {
    let (reset_reg, reset_value) = {
        let info = super::ACPI_INFO.lock();
        match info.as_ref() {
            Some(info) => (info.reset_reg, info.reset_value.unwrap_or(0)),
            None => (None, 0),
        }
    };

    if let Some(reg) = reset_reg {
        unsafe {
            match reg.address_space {
                // System memory
                0 => {
                    let hhdm = crate::memory::paging::HHDM_OFFSET
                        .load(core::sync::atomic::Ordering::Relaxed);
                    let ptr = (hhdm + reg.address) as *mut u8;
                    match reg.bit_width {
                        8 => core::ptr::write_volatile(ptr, reset_value),
                        16 => core::ptr::write_volatile(ptr as *mut u16, reset_value as u16),
                        32 => core::ptr::write_volatile(ptr as *mut u32, reset_value as u32),
                        _ => {}
                    }
                }
                // I/O space
                1 => match reg.bit_width {
                    8 => outb(reg.address as u16, reset_value),
                    16 => outw(reg.address as u16, reset_value as u16),
                    32 => outl(reg.address as u16, reset_value as u32),
                    _ => {}
                },
                _ => {}
            }
        }
        io_delay();
    }

    unsafe {
        // Chipset reset: сначала 0x02 (сигнал), затем 0x06 (полный сброс).
        outb(0xCF9, 0x02);
        io_delay();
        outb(0xCF9, 0x06);
        io_delay();

        // ACPI reset (бит 0 регистра 0x92). Биты A20 не трогаем.
        outb(0x92, 0x01);
        io_delay();

        // i8042 reset: ждём пустой input buffer.
        for _ in 0..100_000u32 {
            if inb(0x64) & 0x02 == 0 {
                break;
            }
        }
        outb(0x64, 0xFE);
        io_delay();
    }

    Err("reset failed (FADT RESET_REG, 0xCF9, 0x92, i8042)")
}

/// Разбирает `PkgLength` AML и возвращает `(длина_заголовка, длина_пакета)`.
///
/// Кодирование: старшие два бита первого байта говорят, сколько байт занимает
/// сама длина (1..4). Младшие биты первого байта — старшая часть значения,
/// остальные байты — младшие.
fn parse_pkg_length(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let first = *bytes.get(at)?;
    let extra = (first >> 6) as usize;
    match extra {
        0 => Some((1, (first & 0x3F) as usize)),
        1 => {
            let second = *bytes.get(at + 1)?;
            Some((2, (((first & 0x0F) as usize) << 8) | second as usize))
        }
        2 => {
            let second = *bytes.get(at + 1)?;
            let third = *bytes.get(at + 2)?;
            Some((
                3,
                (((first & 0x0F) as usize) << 16) | ((second as usize) << 8) | third as usize,
            ))
        }
        _ => {
            let second = *bytes.get(at + 1)?;
            let third = *bytes.get(at + 2)?;
            let fourth = *bytes.get(at + 3)?;
            Some((
                4,
                (((first & 0x0F) as usize) << 24)
                    | ((second as usize) << 16)
                    | ((third as usize) << 8)
                    | fourth as usize,
            ))
        }
    }
}

/// Ищет пакет `_S5` в DSDT и возвращает его SLP_TYP.
///
/// Раскладка, которую разбираем:
///
/// ```text
/// 08 5F 53 35 5F        NameOp  "_S5_"
/// 12 <pkglen>           PackageOp + длина
/// 0A                    NumElements (byte prefix, обычно 2)
/// 0A <slp_typa>         BytePrefix + значение для PM1a
/// 0A <slp_typb>         BytePrefix + значение для PM1b (необязательно)
/// ```
///
/// Значение для PM1b намеренно игнорируется: блок PM1b есть только у
/// двухчипсетных машин, и там обычно объявлен тот же тип сна.
pub fn parse_s5_slp_typ(dsdt_phys: u64) -> Option<u16> {
    let hhdm = crate::memory::paging::HHDM_OFFSET
        .load(core::sync::atomic::Ordering::Relaxed);

    let header = unsafe { &*((hhdm + dsdt_phys) as *const SdtHeader) };
    if &header.signature != b"DSDT" {
        crate::println!(
            "[ACPI] DSDT at {:#x}: bad signature {:?}",
            dsdt_phys,
            core::str::from_utf8(&header.signature).unwrap_or("?")
        );
        return None;
    }
    let length = header.length as usize;
    // Разумный предел: DSDT больше 1 МиБ не бывает, а битая длина увела бы
    // чтение за пределы отображённой памяти.
    if length < core::mem::size_of::<SdtHeader>() || length > 1024 * 1024 {
        crate::println!("[ACPI] DSDT: implausible length {}", length);
        return None;
    }

    let mut bytes = alloc::vec![0u8; length];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = unsafe { core::ptr::read_volatile((hhdm + dsdt_phys + index as u64) as *const u8) };
    }

    // Ищем сигнатуру `_S5_`. Она встречается и в других местах (например, в
    // именах методов), поэтому проверяем, что за ней идёт PackageOp.
    let signature = [b'_', b'S', b'5', b'_'];
    let mut index = 0;
    let mut occurrences = 0;
    while index + 4 <= bytes.len() {
        if bytes[index..index + 4] != signature {
            index += 1;
            continue;
        }
        occurrences += 1;

        let after = index + 4;
        if bytes.get(after) != Some(&0x12) {
            index += 4;
            continue;
        }

        let Some((header_len, pkg_len)) = parse_pkg_length(&bytes, after + 1) else {
            index += 4;
            continue;
        };

        // За длиной пакета идёт NumElements, затем элементы.
        let mut cursor = after + 1 + header_len + 1;
        // PkgLength считает себя: пакет занимает `pkg_len` байт начиная с
        // байта длины, поэтому граница — `after + 1 + pkg_len`.
        let limit = (after + 1 + pkg_len).min(bytes.len());
        while cursor < limit {
            match bytes[cursor] {
                // BytePrefix: следующая байт-константа.
                0x0A => return Some(bytes.get(cursor + 1).copied().unwrap_or(0) as u16),
                // Zero и One — однобайтовые константы. QEMU, например,
                // объявляет `Name (_S5, Package (4) { 0, 0, 0, 0 })`, то есть
                // именно `Zero`, а не `0x0A 0x05`. Без этой ветки разбор
                // не находил SLP_TYP и выключение уходило в fallback.
                0x00 => return Some(0),
                0x01 => return Some(1),
                // WordPrefix и DWordPrefix в `_S5` не встречаются, но если
                // прошивка их использует — читаем значение по размеру.
                0x0B => {
                    let low = bytes.get(cursor + 1).copied().unwrap_or(0) as u16;
                    let high = bytes.get(cursor + 2).copied().unwrap_or(0) as u16;
                    return Some(low | (high << 8));
                }
                other => {
                    crate::println!(
                        "[ACPI] _S5_ at {}: unexpected element prefix 0x{:02x}",
                        index,
                        other
                    );
                    break;
                }
            }
        }

        index += 4;
    }

    crate::println!(
        "[ACPI] DSDT len={}, `_S5_` occurrences={}",
        length,
        occurrences
    );
    None
}

/// Выключает питание машины.
///
/// При успехе функция не возвращается: машина уходит в S5, и питание
/// снимается. Возврат `Err` означает, что ни один способ не сработал, и
/// вызывающий должен напечатать причину и остановиться.
///
/// Порядок попыток:
/// 1. ACPI S5 через PM1a_CNT (и PM1b_CNT, если блок есть);
/// 2. те же порты, но с SLP_TYP по умолчанию — на случай, если `_S5` не
///    разобрался;
/// 3. известные порты эмуляторов (q35, i440fx, VirtualBox);
/// 4. ACPI reset через 0x92 — лучше перезагрузка, чем вечное `hlt`.
pub fn power_off() -> Result<(), &'static str> {
    // Копируем нужные поля и сразу отпускаем блокировку: дальше идёт
    // длинная последовательность портовых операций.
    let (pm1a, pm1b, slp_typ, smi_cmd, acpi_enable) = {
        let info = super::ACPI_INFO.lock();
        let Some(info) = info.as_ref() else {
            return fallback_power_off();
        };
        (
            info.pm1a_cnt_blk,
            info.pm1b_cnt_blk,
            info.s5_slp_typ.unwrap_or(DEFAULT_S5_SLP_TYP),
            info.smi_cmd,
            info.acpi_enable,
        )
    };

    let Some(pm1a) = pm1a else {
        return fallback_power_off();
    };

    unsafe {
        // ACPI должен быть включён (SCI_EN), иначе запись в PM1_CNT
        // игнорируется чипсетом. На части машин SCI_EN поднимает только SMI.
        let control = inw(pm1a as u16);
        if control & SCI_EN == 0 {
            if let (Some(smi), Some(enable)) = (smi_cmd, acpi_enable) {
                outb(smi as u16, enable);
                for _ in 0..100_000 {
                    core::hint::spin_loop();
                    if inw(pm1a as u16) & SCI_EN != 0 {
                        break;
                    }
                }
            }
        }

        // Основная попытка: SLP_TYP из `_S5`.
        let request = (slp_typ << 10) | SLP_EN;
        outw(pm1a as u16, request);
        if let Some(pm1b) = pm1b {
            outw(pm1b as u16, request);
        }

        // Чипсету нужно несколько сотен микросекунд, чтобы снять питание.
        for _ in 0..2_000_000 {
            core::hint::spin_loop();
        }

        // Вторая попытка: тот же порт, но с типом сна по умолчанию. Полезно,
        // если `_S5` в DSDT объявлен не байтом или разобрался неверно.
        if slp_typ != DEFAULT_S5_SLP_TYP {
            let request = (DEFAULT_S5_SLP_TYP << 10) | SLP_EN;
            outw(pm1a as u16, request);
            if let Some(pm1b) = pm1b {
                outw(pm1b as u16, request);
            }
            for _ in 0..2_000_000 {
                core::hint::spin_loop();
            }
        }
    }

    fallback_power_off()
}

/// Порты, которые используют эмуляторы и часть чипсетов.
///
/// Нужны, когда ACPI-таблиц нет вовсе или PM1_CNT не отвечает: в QEMU без
/// FADT `quit` должен всё равно выключать гостя.
fn fallback_power_off() -> Result<(), &'static str> {
    const PORTS: [(u16, u16); 3] = [
        (0x604, 0x2000),  // q35 PM1a_CNT
        (0xB004, 0x2000), // i440fx PM1a_CNT
        (0x4004, 0x3400), // VirtualBox PM1a_CNT
    ];

    unsafe {
        for (port, value) in PORTS {
            outw(port, value);
        }
        for _ in 0..2_000_000 {
            core::hint::spin_loop();
        }

        // Последний вариант — ACPI reset: бит 0 регистра 0x92.
        // Питание не снимется, но машина хотя бы перезагрузится вместо
        // бесконечного hlt.
        outb(0x92, 0x03);
        for _ in 0..2_000_000 {
            core::hint::spin_loop();
        }
    }

    Err("no working poweroff path (ACPI S5, q35, i440fx, VirtualBox, reset)")
}

/// Диагностика для `diag acpi`: какие поля FADT пригодны для выключения.
pub fn describe() -> alloc::string::String {
    use alloc::format;
    let info = super::ACPI_INFO.lock();
    let Some(info) = info.as_ref() else {
        return alloc::string::String::from("ACPI info not available");
    };
    format!(
        "PM1a_CNT={} PM1b_CNT={} _S5 SLP_TYP={} DSDT={}",
        match info.pm1a_cnt_blk {
            Some(p) => format!("{:#06x}", p),
            None => alloc::string::String::from("none"),
        },
        match info.pm1b_cnt_blk {
            Some(p) => format!("{:#06x}", p),
            None => alloc::string::String::from("none"),
        },
        match info.s5_slp_typ {
            Some(t) => format!("{}", t),
            None => alloc::string::String::from("not found"),
        },
        match info.dsdt_phys {
            Some(p) => format!("{:#x}", p),
            None => alloc::string::String::from("none"),
        }
    )
}
