// SMBIOS — паспорт машины: BIOS, система, плата, CPU, модули памяти.
//
// Источник — SMBIOS entry point: сначала указатели из Limine
// (`SmbiosRequest`), затем legacy-скан области 0xF0000..0xFFFFF (её
// просматривают все BIOS/UEFI, включая OVMF).
//
// Таблица структур адресуется физически, поэтому читаем её через HHDM.
// Строки в SMBIOS лежат после форматированной части записи и заканчиваются
// двойным NUL; индекс 0 означает «нет строки».

use crate::memory::paging::HHDM_OFFSET;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

/// Один модуль памяти (SMBIOS type 17).
#[derive(Clone)]
pub struct MemoryDevice {
    pub locator: String,
    pub size_mb: u64,
    pub speed_mts: u16,
    pub memory_type: String,
    pub manufacturer: String,
    pub part_number: String,
}

/// Собранная информация SMBIOS.
#[derive(Clone, Default)]
pub struct SmbiosInfo {
    pub entry_version: u8,
    pub structure_count: u16,
    pub bios_vendor: String,
    pub bios_version: String,
    pub bios_date: String,
    pub system_manufacturer: String,
    pub system_product: String,
    pub system_version: String,
    pub system_serial: String,
    pub board_manufacturer: String,
    pub board_product: String,
    pub cpu_vendor: String,
    pub cpu_version: String,
    pub cpu_cores: u16,
    pub cpu_threads: u16,
    pub memory: Vec<MemoryDevice>,
}

fn hhdm() -> usize {
    HHDM_OFFSET.load(Ordering::Relaxed) as usize
}

fn u8_at(addr: usize, offset: usize) -> u8 {
    unsafe { core::ptr::read_unaligned((addr + offset) as *const u8) }
}

fn u16_at(addr: usize, offset: usize) -> u16 {
    unsafe { core::ptr::read_unaligned((addr + offset) as *const u16) }
}

fn u32_at(addr: usize, offset: usize) -> u32 {
    unsafe { core::ptr::read_unaligned((addr + offset) as *const u32) }
}

fn u64_at(addr: usize, offset: usize) -> u64 {
    unsafe { core::ptr::read_unaligned((addr + offset) as *const u64) }
}

fn bytes_at(addr: usize, len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(addr as *const u8, len) }
}

/// Разбирает entry point по сигнатуре. Возвращает (виртуальный адрес таблицы,
/// число структур; 0 = до type 127).
fn parse_entry(addr: usize) -> Option<(usize, u16)> {
    let signature = bytes_at(addr, 5);
    if signature == b"_SM3_" {
        // SMBIOS 3.0: Table Address (u64) по смещению 0x10, физический.
        let table = u64_at(addr, 0x10) as usize;
        if table == 0 {
            return None;
        }
        return Some((hhdm() + table, 0));
    }
    if signature == b"_SM_" {
        // SMBIOS 2.x: Table Address (u32) по 0x18, Number of Structures по 0x1C.
        let table = u32_at(addr, 0x18) as usize;
        let count = u16_at(addr, 0x1C);
        if table == 0 {
            return None;
        }
        return Some((hhdm() + table, count));
    }
    None
}

/// Legacy-скан BIOS-области 0xF0000..0xFFFFF с шагом 16 байт.
fn scan_bios_area() -> Option<(usize, u16)> {
    let base = hhdm() + 0xF_0000;
    let mut addr = base;
    while addr < base + 0x1_0000 {
        if let Some(found) = parse_entry(addr) {
            return Some(found);
        }
        addr += 16;
    }
    None
}

/// Точка входа SMBIOS: Limine → legacy-скан.
fn entry_point() -> Option<(usize, u16)> {
    if let Some(response) = crate::SMBIOS_REQUEST.response() {
        for pointer in [response.entry_32, response.entry_64] {
            if pointer.is_null() {
                continue;
            }
            // Limine отдаёт виртуальный адрес (HHDM), но для base revision 3/4
            // это физический адрес. Различаем по величине: физическая память
            // заведомо меньше HHDM-базы.
            let raw = pointer as usize;
            let base = hhdm();
            let candidate = if raw >= base {
                raw
            } else {
                base.wrapping_add(raw)
            };
            if let Some(found) = parse_entry(candidate) {
                return Some(found);
            }
        }
    }
    scan_bios_area()
}

/// Строка по индексу из таблицы строк записи (индекс 1-based).
fn string_at(strings: usize, index: u8) -> String {
    if index == 0 {
        return String::new();
    }

    let mut pointer = strings;
    let mut current = 1usize;
    loop {
        let mut text = String::new();
        loop {
            let byte = u8_at(pointer, 0);
            if byte == 0 {
                break;
            }
            if (0x20..0x7F).contains(&byte) {
                text.push(byte as char);
            }
            pointer += 1;
        }

        if current == index as usize {
            return text;
        }
        if text.is_empty() {
            return String::new();
        }
        // Пропускаем завершающий NUL и переходим к следующей строке.
        pointer += 1;
        current += 1;
        if current > 128 {
            return String::new();
        }
    }
}

/// Обходит все структуры и вызывает `visit(type, formatted_addr, strings_addr)`.
fn walk(table: usize, count: u16, mut visit: impl FnMut(u8, usize, usize)) {
    let mut addr = table;
    let mut seen = 0u32;

    loop {
        let kind = u8_at(addr, 0);
        let length = u8_at(addr, 1) as usize;
        if length < 4 {
            break;
        }
        let strings = addr + length;
        visit(kind, addr, strings);

        // Конец таблицы строк — двойной NUL.
        let mut pointer = strings;
        let limit = strings + 4096;
        loop {
            if pointer >= limit {
                pointer = strings;
                break;
            }
            if u8_at(pointer, 0) == 0 && u8_at(pointer, 1) == 0 {
                pointer += 2;
                break;
            }
            pointer += 1;
        }

        addr = pointer;
        seen += 1;
        if kind == 127 || seen > 512 {
            break;
        }
        if count != 0 && seen >= count as u32 {
            break;
        }
    }
}

fn memory_type_name(value: u8) -> String {
    String::from(match value {
        0x01 => "Other",
        0x03 => "DRAM",
        0x0F | 0x11 => "SDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => "unknown",
    })
}

/// Читает SMBIOS. `None`, если entry point не найден.
pub fn read() -> Option<SmbiosInfo> {
    let (table, count) = entry_point()?;

    let mut info = SmbiosInfo {
        structure_count: count,
        ..SmbiosInfo::default()
    };

    walk(table, count, |kind, formatted, strings| match kind {
        0 => {
            // BIOS Information
            info.bios_vendor = string_at(strings, u8_at(formatted, 0x04));
            info.bios_version = string_at(strings, u8_at(formatted, 0x05));
            info.bios_date = string_at(strings, u8_at(formatted, 0x08));
        }
        1 => {
            // System Information
            info.system_manufacturer = string_at(strings, u8_at(formatted, 0x04));
            info.system_product = string_at(strings, u8_at(formatted, 0x05));
            info.system_version = string_at(strings, u8_at(formatted, 0x06));
            info.system_serial = string_at(strings, u8_at(formatted, 0x07));
        }
        2 => {
            // Baseboard Information
            info.board_manufacturer = string_at(strings, u8_at(formatted, 0x04));
            info.board_product = string_at(strings, u8_at(formatted, 0x05));
        }
        4 => {
            // Processor Information
            info.cpu_vendor = string_at(strings, u8_at(formatted, 0x07));
            info.cpu_version = string_at(strings, u8_at(formatted, 0x10));
            let length = u8_at(formatted, 0x01) as usize;
            if info.cpu_cores == 0 && length > 0x23 {
                info.cpu_cores = u16_at(formatted, 0x23);
            }
            if info.cpu_threads == 0 && length > 0x25 {
                info.cpu_threads = u16_at(formatted, 0x25);
            }
        }
        17 => {
            // Memory Device
            let size = u16_at(formatted, 0x0C);
            if size == 0 || size == 0xFFFF {
                return;
            }
            let size_mb = if size == 0x7FFF {
                u32_at(formatted, 0x1C) as u64
            } else if size & 0x8000 != 0 {
                // Размер в килобайтах.
                ((size & 0x7FFF) as u64) / 1024
            } else {
                size as u64
            };
            if size_mb == 0 {
                return;
            }

            info.memory.push(MemoryDevice {
                locator: string_at(strings, u8_at(formatted, 0x10)),
                size_mb,
                speed_mts: u16_at(formatted, 0x15),
                memory_type: memory_type_name(u8_at(formatted, 0x12)),
                manufacturer: string_at(strings, u8_at(formatted, 0x17)),
                part_number: string_at(strings, u8_at(formatted, 0x1A)),
            });
        }
        _ => {}
    });

    Some(info)
}

/// Печатает краткую сводку SMBIOS в boot-лог.
pub fn init() {
    let Some(info) = read() else {
        crate::println!("[SMBIOS] entry point not found");
        return;
    };

    crate::println!(
        "[SMBIOS] {} / {} ({})",
        if info.system_manufacturer.is_empty() { "unknown" } else { &info.system_manufacturer },
        if info.system_product.is_empty() { "unknown" } else { &info.system_product },
        if info.system_version.is_empty() { "-" } else { &info.system_version }
    );
    crate::println!("[SMBIOS] board: {} {}", info.board_manufacturer, info.board_product);
    crate::println!(
        "[SMBIOS] CPU: {} {} ({} cores / {} threads)",
        info.cpu_vendor,
        info.cpu_version,
        info.cpu_cores,
        info.cpu_threads
    );
    crate::println!(
        "[SMBIOS] BIOS: {} {} ({})",
        info.bios_vendor,
        info.bios_version,
        info.bios_date
    );

    let mut total_mb = 0u64;
    for device in &info.memory {
        total_mb += device.size_mb;
    }
    crate::println!(
        "[SMBIOS] memory: {} module(s), {} MiB total",
        info.memory.len(),
        total_mb
    );
}
