// AHCI (Advanced Host Controller Interface) — SATA disk driver.
//
// AHCI — стандартный интерфейс для SATA контроллеров.
// Есть на всех современных PC (Intel, AMD chipsets).
// Использует Memory-Mapped Registers + Command Lists + FIS.
//
// Регистры HBA (Host Bus Adapter) доступны через BAR5 (ABAR).
// Каждый порт имеет Command List + FIS接收区 + таблицу散列.

use crate::println;
use alloc::vec::Vec;
use alloc::string::String;
use spin::Mutex;
use core::sync::atomic::{AtomicBool, Ordering};

/// AHCI HBA Registers (offsets от ABAR)
const HBA_CAP: u64 = 0x00;      // Host Capabilities
const HBA_GHC: u64 = 0x04;      // Global Host Control
const HBA_IS: u64 = 0x08;       // Interrupt Status
const HBA_PI: u64 = 0x0C;       // Ports Implemented
const HBA_CMD: u64 = 0x18;      // Command and Status
const HBA_SCTL: u64 = 0x44;     // Serial ATA Control

// Port registers (offset = 0x100 + port_num * 0x80)
const PORT_CLB: u64 = 0x00;     // Command List Base Address
const PORT_FB: u64 = 0x08;      // FIS Base Address
const PORT_IS: u64 = 0x10;      // Interrupt Status
const PORT_CMD: u64 = 0x18;     // Command and Status
const PORT_TFD: u64 = 0x20;     // Task File Data
const PORT_SIG: u64 = 0x24;     // Signature
const PORT_SSTS: u64 = 0x28;    // SATA Status
const PORT_SERR: u64 = 0x30;    // SATA Error
const PORT_CI: u64 = 0x38;      // Command Issue

// FIS types
const FIS_TYPE_REG_H2D: u8 = 0x27;
const FIS_TYPE_REG_D2H: u8 = 0x34;
const FIS_TYPE_DATA: u8 = 0x46;

// ATA commands
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

// Port CMD bits
const PORT_CMD_ST: u32 = 1;        // Start
const PORT_CMD_FRE: u32 = 1 << 4;  // FIS Receive Enable
const PORT_CMD_CLO: u32 = 1 << 3;  // Command List Overrun
const PORT_CMD_FR: u32 = 1 << 14;  // FIS Receive Running
const PORT_CMD_CR: u32 = 1 << 15;  // Command List Running

// Port TFD bits
const TFD_BSY: u8 = 1 << 7;
const TFD_DRQ: u8 = 1 << 3;

// HBA CAP bits
const HBA_CAP_S64A: u32 = 1 << 31; // Supports 64-bit Addressing
const HBA_CAP_SIS: u32 = 1 << 28;  // Supports Interface Speed Control
const HBA_CAP_SMPS: u32 = 1 << 27; // Supports Mechanical Presence Switch
const HBA_CAP_NP: u32 = (1u32 << 5) - 1; // Number of Ports (bits 4:0)

// GHCI bits
const GHC_HR: u32 = 1;     // HBA Reset
const GHC_IE: u32 = 1 << 1; // Interrupt Enable
const GHC_AE: u32 = 1 << 31; // AHCI Enable: без него контроллер работает в IDE-режиме

/// HBA Memory (MMIO region at BAR5)
static HBA_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

unsafe fn hba_read32(offset: u64) -> u32 {
    let base = HBA_BASE.load(Ordering::Relaxed);
    core::ptr::read_volatile((base + offset) as *const u32)
}

unsafe fn hba_write32(offset: u64, val: u32) {
    let base = HBA_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

unsafe fn port_read32(port: u32, offset: u64) -> u32 {
    hba_read32(0x100 + (port as u64) * 0x80 + offset)
}

unsafe fn port_write32(port: u32, offset: u64, val: u32) {
    hba_write32(0x100 + (port as u64) * 0x80 + offset, val);
}

/// Command Header — описывает одну ATA команду (32 байта, AHCI 1.3 §4.2.2).
///
/// ВАЖНО: раньше структура содержала поля FIS (features/lba/device/sector_count)
/// прямо в заголовке. Из-за этого PRDTL и CTBA попадали не на свои смещения
/// (2 и 8), контроллер видел PRDTL=0/CTBA=0 и любая команда (IDENTIFY, READ,
/// WRITE) завершалась таймаутом. Поля FIS живут только в command table.
#[repr(C)]
struct CommandHeader {
    /// DW0, байт 0: CFL[4:0], W[6], P[7].
    cfl_w: u8,
    /// DW0, байт 1: A[0], R[1], B[2], C[3], PMP[7:4].
    flags: u8,
    /// DW1: Physical Region Descriptor Table Length.
    prdtl: u16,
    /// DW2: Physical Region Descriptor Byte Count (заполняет контроллер).
    prdbc: u32,
    /// DW3: Command Table Base Address (младшие 32 бита).
    ctba: u32,
    /// DW4: Command Table Base Address (старшие 32 бита).
    ctbau: u32,
    /// DW5..DW8: reserved.
    _reserved: [u32; 4],
}

/// FIS Register H2D — Host to Device FIS (AHCI 1.3 §10.3.3, 20 байт).
///
/// Раскладка строго побайтовая: 0 = тип, 1 = C|PMP, 2 = команда, 3 = Feature,
/// 4..6 = LBA0..2, 7 = Device, 8..10 = LBA3..5, 11 = Feature (high),
/// 12..13 = Count, 14 = ICC, 15 = Control.
///
/// Раньше между `pmport` и `command` был лишний `_reserved0`, из-за чего все
/// поля уезжали на байт: устройство получало команду 0x00 и отвечало ошибкой
/// (TFD.ERR=1), а PxCI не сбрасывался — «identify command timeout».
#[repr(C)]
struct FisRegH2d {
    fis_type: u8,     // 0: FIS_TYPE_REG_H2D = 0x27
    c_pmp: u8,        // 1: бит 7 = C (command), биты 3..0 = PM port
    command: u8,      // 2: ATA command
    features: u8,     // 3: Feature (low)
    lba_low: u8,      // 4: LBA0
    lba_mid: u8,      // 5: LBA1
    lba_high: u8,     // 6: LBA2
    device: u8,       // 7: Device
    lba_low_exp: u8,  // 8: LBA3
    lba_mid_exp: u8,  // 9: LBA4
    lba_high_exp: u8, // 10: LBA5
    features_exp: u8, // 11: Feature (high)
    sector_count_lo: u8, // 12
    sector_count_hi: u8, // 13
    icc: u8,          // 14: Isochronous Command Completion
    control: u8,      // 15
    _reserved: [u8; 4], // 16..20
}

/// PRD Entry — Physical Region Descriptor
///
/// DBA (base_addr) + DBAU (base_addr_hi) образуют 64-битный физический адрес.
/// Раньше старшая половина не заполнялась, поэтому на машинах с >4 ГБ RAM
/// (или с high-mem буферами) DMA уходил в мусорный адрес.
#[repr(C)]
struct PrdEntry {
    base_addr: u32,
    base_addr_hi: u32,
    _reserved: u32,
    byte_count: u32,  // bit 31 = interrupt on completion
}

/// AHCI Port info
struct AhciPort {
    num: u32,
    active: bool,
    ssts: u32,
}

static AHCI_PORTS: Mutex<Vec<AhciPort>> = Mutex::new(Vec::new());

// DMA-адрес для буферов (используем HHDM — физическая = виртуальная - hhdm)
fn virt_to_phys(virt: u64) -> u64 {
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    virt - hhdm
}

/// Инициализация AHCI контроллера
pub fn init() {
    // Ищем AHCI контроллер через PCI (class 0x01, subclass 0x06)
    let ahci_device = match super::pci::find_device(0x01, 0x06) {
        Some(d) => d,
        None => {
            println!("[AHCI] No AHCI controller found");
            return;
        }
    };

    println!("[AHCI] Found controller at PCI [{:02X}:{:02X}.{}]",
        ahci_device.bus, ahci_device.dev, ahci_device.func);
    println!("[AHCI]   Vendor: {:04X} Device: {:04X}",
        ahci_device.vendor_id, ahci_device.device_id);

    // Включаем Bus Master и Memory Space
    unsafe {
        super::pci::enable_bus_master(ahci_device.bus, ahci_device.dev, ahci_device.func);
    }

    // BAR5 (ABAR) — HBA Memory Registers
    let abar = ahci_device.bar5 & 0xFFFFFFF0;
    if abar == 0 {
        println!("[AHCI] ERROR: BAR5 is zero — controller not configured");
        return;
    }

    // MMIO: abar через HHDM
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let hba_virt = hhdm + abar as u64;
    HBA_BASE.store(hba_virt, Ordering::SeqCst);

    println!("[AHCI] ABAR: phys={:#x} virt={:#x}", abar, hba_virt);

    // Читаем HBA capabilities
    unsafe {
        let cap = hba_read32(HBA_CAP);
        let num_ports = (cap & HBA_CAP_NP) as u8 + 1;
        let has_64bit = cap & HBA_CAP_S64A != 0;
        println!("[AHCI] CAP: {} ports, 64-bit={}", num_ports, has_64bit);

        // Port Implemented
        let pi = hba_read32(HBA_PI);
        println!("[AHCI] PI: {:#032b}", pi);

        // HBA Reset (GHCR bit 0) — нужен для инициализации
        let ghc = hba_read32(HBA_GHC);
        if ghc & GHC_HR == 0 {
            // First init: reset HBA
            hba_write32(HBA_GHC, GHC_HR);
            // Ждём пока reset завершится
            let mut timeout = 1_000_000u32;
            while hba_read32(HBA_GHC) & GHC_HR != 0 && timeout > 0 {
                timeout -= 1;
            }
            if timeout == 0 {
                println!("[AHCI] WARNING: HBA reset timeout");
            }
        }

        // Включаем AHCI-режим и прерывания.
        hba_write32(HBA_GHC, hba_read32(HBA_GHC) | GHC_AE | GHC_IE);

        // Сканируем порты
        let mut ports = AHCI_PORTS.lock();
        for i in 0..num_ports {
            if pi & (1 << i) == 0 { continue; }
            let i = i as u32;

            let ssts = port_read32(i, PORT_SSTS);
            let det = ssts & 0x0F; // Device Detection
            let ipm = (ssts >> 8) & 0x0F; // Interface Power Management

            if det != 0x03 || ipm != 0x01 {
                println!("[AHCI] Port {}: no device (DET={}, IPM={})", i, det, ipm);
                continue;
            }

            let sig = port_read32(i, PORT_SIG);
            let sig_str = match sig >> 16 {
                0x0000 => "ATA",
                0xEB14 => "ATAPI",
                0x9669 => "SEMB",
                0xC33C => "SATA PM",
                _ => "ATA/SATA",
            };

            // PxSIG на части контроллеров (в т.ч. ICH9 в QEMU) читается как
            // 0xFFFFFFFF даже при подключённом диске, поэтому «фантомность»
            // проверяем не по SIG, а командой IDENTIFY DEVICE.
            init_port(i);

            match identify_device(i) {
                Some(info) => {
                    println!(
                        "[AHCI] Port {}: {} ({:#010x}), {} секторов",
                        i, sig_str, sig, info.total_sectors
                    );
                    DISK_INFO.lock().push(info);
                    ports.push(AhciPort { num: i, active: true, ssts });
                }
                None => {
                    println!("[AHCI] Port {}: нет ответа на IDENTIFY (SIG={:#010x}), skip", i, sig);
                }
            }
        }

        println!("[AHCI] {} active ports", ports.len());
    }
}

/// Инициализация одного AHCI порта
unsafe fn init_port(port: u32) {
    // Останавливаем порт
    port_stop(port);

    // Выделяем Command List (1K выровненный, 32 командных заголовка)
    let cmd_list_size = core::mem::size_of::<CommandHeader>() * 32; // 32 cmds
    let cmd_list_layout = core::alloc::Layout::from_size_align(cmd_list_size, 1024).unwrap();
    let cmd_list_ptr = alloc::alloc::alloc_zeroed(cmd_list_layout);
    if cmd_list_ptr.is_null() { return; }
    let cmd_list_phys = virt_to_phys(cmd_list_ptr as u64);

    // FIS接收区 (256 bytes, выровнено)
    let fis_size = 256;
    let fis_layout = core::alloc::Layout::from_size_align(fis_size, 256).unwrap();
    let fis_ptr = alloc::alloc::alloc_zeroed(fis_layout);
    if fis_ptr.is_null() {
        alloc::alloc::dealloc(cmd_list_ptr, cmd_list_layout);
        return;
    }
    let fis_phys = virt_to_phys(fis_ptr as u64);

    // Устанавливаем адреса
    port_write32(port, PORT_CLB, cmd_list_phys as u32);
    port_write32(port, PORT_CLB + 4, (cmd_list_phys >> 32) as u32);
    port_write32(port, PORT_FB, fis_phys as u32);
    port_write32(port, PORT_FB + 4, (fis_phys >> 32) as u32);

    // Включаем FIS Receive и Start
    let cmd = port_read32(port, PORT_CMD);
    port_write32(port, PORT_CMD, cmd | PORT_CMD_FRE | PORT_CMD_ST);

    // Очищаем ошибки
    port_write32(port, PORT_SERR, 0xFFFFFFFF);
    port_write32(port, PORT_IS, 0xFFFFFFFF);
}

/// Остановить порт
unsafe fn port_stop(port: u32) {
    let mut cmd = port_read32(port, PORT_CMD);
    cmd &= !(PORT_CMD_ST | PORT_CMD_FRE);
    port_write32(port, PORT_CMD, cmd);

    // Ждём пока CR и FR станут 0
    let mut timeout = 500_000u32;
    while port_read32(port, PORT_CMD) & (PORT_CMD_FR | PORT_CMD_CR) != 0 && timeout > 0 {
        timeout -= 1;
    }
}

/// Проверить, что команда завершилась без ошибки.
///
/// PxIS.DHRS (бит 0) выставляется при ЛЮБОМ завершении команды и ошибкой НЕ
/// является — раньше именно эта проверка отбрасывала успешный IDENTIFY.
/// Ошибку показывает PxTFD.ERR (бит 0), а интерфейсные сбои — старшие биты
/// PxIS (TFE/HBF/IFS/HBDS).
unsafe fn command_error(port_num: u32) -> Option<u32> {
    let tfd = port_read32(port_num, PORT_TFD);
    let is = port_read32(port_num, PORT_IS);
    port_write32(port_num, PORT_IS, 0xFFFFFFFF);
    if tfd & 0x01 != 0 || is & 0x7800_0000 != 0 {
        Some(is)
    } else {
        None
    }
}

/// Инициализировать Register H2D FIS для команды.
///
/// Бит 7 байта 1 (`pmport`) — это C (command) бит. Без него контроллер
/// трактует FIS как control-запись, устройство молча игнорирует команду и
/// PxCI никогда не сбрасывается — отсюда «identify command timeout».
unsafe fn fis_command(fis: &mut FisRegH2d, command: u8) {
    fis.fis_type = FIS_TYPE_REG_H2D;
    fis.c_pmp = 0x80; // C = 1, port multiplier = 0
    fis.command = command;
}

/// Заполнить Command Header для одной команды с одним PRD.
///
/// `write` выставляет бит W в DW0 (0 — чтение из устройства, 1 — запись).
unsafe fn fill_cmd_header(cmd: &mut CommandHeader, write: bool, ct_phys: u64) {
    cmd.cfl_w = 5 | if write { 0x40 } else { 0 }; // CFL=5 dwords, W при записи
    cmd.flags = 0x08;                             // C=1: сбросить PxCI по завершении
    cmd.prdtl = 1;                                // один PRD
    cmd.prdbc = 0;                                // заполнит контроллер
    cmd.ctba = ct_phys as u32;
    cmd.ctbau = (ct_phys >> 32) as u32;
}

/// Прочитать сектор с AHCI порта
pub fn read_sectors(port_num: u32, lba: u64, count: u16, buffer: &mut [u8]) -> bool {
    unsafe {
        // Ждём пока порт свободен (BSY=0, DRQ=0)
        let mut timeout = 500_000u32;
        while port_read32(port_num, PORT_TFD) as u8 & (TFD_BSY | TFD_DRQ) != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 { return false; }

        // Clear error
        port_write32(port_num, PORT_IS, 0xFFFFFFFF);

        // Command List entry 0
        let cmd_list_phys = port_read32(port_num, PORT_CLB) as u64
            | ((port_read32(port_num, PORT_CLB + 4) as u64) << 32);
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
        let cmd_list = (hhdm + cmd_list_phys) as *mut CommandHeader;
        let cmd = &mut *cmd_list;

        // Command Table для FIS (выделяем на стеке — маленький)
        let ct_layout = core::alloc::Layout::from_size_align(256, 256).unwrap();
        let ct_ptr = alloc::alloc::alloc_zeroed(ct_layout);
        if ct_ptr.is_null() { return false; }
        let ct_phys = virt_to_phys(ct_ptr as u64);

        // Заполняем FIS
        let fis = ct_ptr as *mut FisRegH2d;
        fis_command(&mut *fis, ATA_CMD_READ_DMA_EXT);
        // LBA48: в поле device биты 27..24 адреса НЕ кладутся (они уходят в
        // lba_low_exp), иначе на дисках >128 ГиБ читается не тот сектор.
        (*fis).device = 1 << 6; // LBA mode
        (*fis).lba_low = lba as u8;
        (*fis).lba_mid = (lba >> 8) as u8;
        (*fis).lba_high = (lba >> 16) as u8;
        (*fis).lba_low_exp = (lba >> 24) as u8;
        (*fis).lba_mid_exp = (lba >> 32) as u8;
        (*fis).lba_high_exp = (lba >> 40) as u8;
        (*fis).sector_count_lo = (count & 0xFF) as u8;
        (*fis).sector_count_hi = ((count >> 8) & 0xFF) as u8;

        // PRD entry — один регион (данные в buffer)
        let buf_phys = virt_to_phys(buffer.as_ptr() as u64);
        let prd = (ct_ptr as *mut u8).add(128) as *mut PrdEntry;
        (*prd).base_addr = buf_phys as u32;
        (*prd).base_addr_hi = (buf_phys >> 32) as u32;
        (*prd).byte_count = (buffer.len() as u32) | (1 << 31); // interrupt on complete

        // Command Header
        fill_cmd_header(cmd, false, ct_phys);

        // Command Issue
        port_write32(port_num, PORT_CI, 1);

        // Ждём завершения
        timeout = 5_000_000;
        while port_read32(port_num, PORT_CI) & 1 != 0 && timeout > 0 {
            timeout -= 1;
        }

        // Освобождаем Command Table
        alloc::alloc::dealloc(ct_ptr, ct_layout);

        if timeout == 0 {
            println!("[AHCI] Port {}: read timeout!", port_num);
            return false;
        }

        // Проверяем ошибки
        if let Some(is) = command_error(port_num) {
            println!("[AHCI] Port {}: read error IS={:#x}", port_num, is);
            return false;
        }

        true
    }
}

/// Количество активных портов
pub fn active_port_count() -> usize {
    AHCI_PORTS.lock().iter().filter(|p| p.active).count()
}

/// Первый активный порт
pub fn first_port() -> Option<u32> {
    AHCI_PORTS.lock().iter().find(|p| p.active).map(|p| p.num)
}

/// Список всех активных портов (для перечисления дисков в blockdev).
pub fn active_ports() -> Vec<u32> {
    AHCI_PORTS.lock().iter().filter(|p| p.active).map(|p| p.num).collect()
}

/// Записать секторы на диск через AHCI
pub fn write_sectors(port_num: u32, lba: u64, count: u16, buffer: &[u8]) -> bool {
    unsafe {
        // Ждём пока порт свободен
        let mut timeout = 500_000u32;
        while port_read32(port_num, PORT_TFD) as u8 & (TFD_BSY | TFD_DRQ) != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 { return false; }

        // Clear error
        port_write32(port_num, PORT_IS, 0xFFFFFFFF);

        // Command List
        let cmd_list_phys = port_read32(port_num, PORT_CLB) as u64
            | ((port_read32(port_num, PORT_CLB + 4) as u64) << 32);
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
        let cmd_list = (hhdm + cmd_list_phys) as *mut CommandHeader;
        let cmd = &mut *cmd_list;

        // Command Table
        let ct_layout = core::alloc::Layout::from_size_align(256, 256).unwrap();
        let ct_ptr = alloc::alloc::alloc_zeroed(ct_layout);
        if ct_ptr.is_null() { return false; }
        let ct_phys = virt_to_phys(ct_ptr as u64);

        // FIS — WRITE DMA EXT
        let fis = ct_ptr as *mut FisRegH2d;
        fis_command(&mut *fis, ATA_CMD_WRITE_DMA_EXT);
        (*fis).device = 1 << 6; // LBA48, см. комментарий в read_sectors
        (*fis).lba_low = lba as u8;
        (*fis).lba_mid = (lba >> 8) as u8;
        (*fis).lba_high = (lba >> 16) as u8;
        (*fis).lba_low_exp = (lba >> 24) as u8;
        (*fis).lba_mid_exp = (lba >> 32) as u8;
        (*fis).lba_high_exp = (lba >> 40) as u8;
        (*fis).sector_count_lo = (count & 0xFF) as u8;
        (*fis).sector_count_hi = ((count >> 8) & 0xFF) as u8;

        // PRD entry
        let buf_phys = virt_to_phys(buffer.as_ptr() as u64);
        let prd = (ct_ptr as *mut u8).add(128) as *mut PrdEntry;
        (*prd).base_addr = buf_phys as u32;
        (*prd).base_addr_hi = (buf_phys >> 32) as u32;
        (*prd).byte_count = (buffer.len() as u32) | (1 << 31);

        // Command Header
        fill_cmd_header(cmd, true, ct_phys);

        // Command Issue
        port_write32(port_num, PORT_CI, 1);

        // Wait
        timeout = 5_000_000;
        while port_read32(port_num, PORT_CI) & 1 != 0 && timeout > 0 {
            timeout -= 1;
        }

        alloc::alloc::dealloc(ct_ptr, ct_layout);

        if timeout == 0 { return false; }
        if command_error(port_num).is_some() { return false; }
        true
    }
}

/// Максимум секторов на одну AHCI-команду.
///
/// PRDT ограничивает один PRD 4 МиБ, но драйвер использует один PRD на команду,
/// поэтому держим кусок в 128 секторов (64 КиБ) — этого достаточно для скорости
/// и гарантированно влезает в любые лимиты.
const AHCI_MAX_SECTORS: usize = 128;

/// Реализация DiskIo для AHCI
///
/// ВАЖНО: DiskIo::read принимает буфер произвольной длины (например, целый
/// кластер FAT32 = 4 КиБ). Раньше здесь всегда запрашивался ровно 1 сектор,
/// из-за чего все сектора кроме первого оставались нулевыми и каталоги/файлы
/// читались «наполовину». Теперь запрос режется на куски.
impl crate::fs::disk::DiskIo for AhciDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        self.read_bytes(sector.saturating_mul(512), buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), crate::fs::vfs::FsError> {
        self.write_bytes(sector.saturating_mul(512), buf)
    }
}

impl AhciDisk {
    fn read_bytes(&mut self, byte_offset: u64, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        let ss = self.sector_size as u64;
        if ss == 0 {
            return Err(crate::fs::vfs::FsError::IoError);
        }
        let lba = byte_offset / ss;
        let in_sector = (byte_offset % ss) as usize;

        if in_sector == 0 && buf.len() % ss as usize == 0 {
            let max_bytes = AHCI_MAX_SECTORS * ss as usize;
            for (i, chunk) in buf.chunks_mut(max_bytes).enumerate() {
                let count = (chunk.len() / ss as usize) as u16;
                if count == 0 || chunk.len() % ss as usize != 0 {
                    return Err(crate::fs::vfs::FsError::InvalidPath);
                }
                let lba = lba + (i * AHCI_MAX_SECTORS) as u64;
                if !read_sectors(self.port, lba, count, chunk) {
                    return Err(crate::fs::vfs::FsError::IoError);
                }
            }
            return Ok(());
        }

        let need = (in_sector + buf.len() + ss as usize - 1) / ss as usize;
        let mut tmp = alloc::vec![0u8; need * ss as usize];
        let max_bytes = AHCI_MAX_SECTORS * ss as usize;
        for (i, chunk) in tmp.chunks_mut(max_bytes).enumerate() {
            let count = (chunk.len() / ss as usize) as u16;
            let lba = lba + (i * AHCI_MAX_SECTORS) as u64;
            if !read_sectors(self.port, lba, count, chunk) {
                return Err(crate::fs::vfs::FsError::IoError);
            }
        }
        buf.copy_from_slice(&tmp[in_sector..in_sector + buf.len()]);
        Ok(())
    }

    fn write_bytes(&mut self, byte_offset: u64, buf: &[u8]) -> Result<(), crate::fs::vfs::FsError> {
        let ss = self.sector_size as u64;
        if ss == 0 {
            return Err(crate::fs::vfs::FsError::IoError);
        }
        let lba = byte_offset / ss;
        let in_sector = (byte_offset % ss) as usize;

        if in_sector == 0 && buf.len() % ss as usize == 0 {
            let max_bytes = AHCI_MAX_SECTORS * ss as usize;
            for (i, chunk) in buf.chunks(max_bytes).enumerate() {
                let count = (chunk.len() / ss as usize) as u16;
                if count == 0 || chunk.len() % ss as usize != 0 {
                    return Err(crate::fs::vfs::FsError::InvalidPath);
                }
                let lba = lba + (i * AHCI_MAX_SECTORS) as u64;
                if !write_sectors(self.port, lba, count, chunk) {
                    return Err(crate::fs::vfs::FsError::IoError);
                }
            }
            return Ok(());
        }

        // Неполный сектор: read-modify-write.
        let need = (in_sector + buf.len() + ss as usize - 1) / ss as usize;
        let mut tmp = alloc::vec![0u8; need * ss as usize];
        let max_bytes = AHCI_MAX_SECTORS * ss as usize;
        for (i, chunk) in tmp.chunks_mut(max_bytes).enumerate() {
            let count = (chunk.len() / ss as usize) as u16;
            let lba = lba + (i * AHCI_MAX_SECTORS) as u64;
            if !read_sectors(self.port, lba, count, chunk) {
                return Err(crate::fs::vfs::FsError::IoError);
            }
        }
        tmp[in_sector..in_sector + buf.len()].copy_from_slice(buf);
        for (i, chunk) in tmp.chunks(max_bytes).enumerate() {
            let count = (chunk.len() / ss as usize) as u16;
            let lba = lba + (i * AHCI_MAX_SECTORS) as u64;
            if !write_sectors(self.port, lba, count, chunk) {
                return Err(crate::fs::vfs::FsError::IoError);
            }
        }
        Ok(())
    }
}

pub struct AhciDisk {
    port: u32,
    sector_size: u16,
}

impl AhciDisk {
    pub fn new(port: u32) -> Self {
        // VFS/boot-log работают в 512-байтных секторах. Для 4Kn-диска нужен
        // реальный размер сектора, иначе GPT/FAT будут читать не с того
        // смещения. `with_sector_size` используется там, где IDENTIFY уже
        // выполнен (blockdev), а здесь — для остальных вызывающих.
        let sector_size = identify_device(port)
            .map(|info| info.sector_size)
            .unwrap_or(512);
        Self::with_sector_size(port, sector_size)
    }

    pub fn with_sector_size(port: u32, sector_size: u16) -> Self {
        Self {
            port,
            sector_size: if sector_size == 0 { 512 } else { sector_size },
        }
    }

    pub fn port(&self) -> u32 {
        self.port
    }
}

/// Информация о диске, полученная через ATA IDENTIFY
#[derive(Clone)]
pub struct DiskInfo {
    pub total_sectors: u64,
    pub sector_size: u16,
    pub serial: String,
    pub model: String,
}

static DISK_INFO: Mutex<Vec<DiskInfo>> = Mutex::new(Vec::new());

/// Получить информацию о найденных дисках
pub fn get_disk_info() -> Vec<DiskInfo> {
    DISK_INFO.lock().clone()
}

/// Выполнить ATA IDENTIFY DEVICE на указанном порту AHCI
pub fn identify_device(port_num: u32) -> Option<DiskInfo> {
    unsafe {
        // Ждём пока порт свободен (BSY=0, DRQ=0)
        let mut timeout = 50_000u32;
        while port_read32(port_num, PORT_TFD) as u8 & (TFD_BSY | TFD_DRQ) != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 {
            println!("[AHCI] Port {}: identify timeout (port busy)", port_num);
            return None;
        }

        // Очи Status
        port_write32(port_num, PORT_IS, 0xFFFFFFFF);

        // Command List
        let cmd_list_phys = port_read32(port_num, PORT_CLB) as u64
            | ((port_read32(port_num, PORT_CLB + 4) as u64) << 32);
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
        let cmd_list = (hhdm + cmd_list_phys) as *mut CommandHeader;
        let cmd = &mut *cmd_list;

        // Command Table (FIS + PRD)
        let ct_layout = core::alloc::Layout::from_size_align(256, 256).unwrap();
        let ct_ptr = alloc::alloc::alloc_zeroed(ct_layout);
        if ct_ptr.is_null() { return None; }
        let ct_phys = virt_to_phys(ct_ptr as u64);

        // Буфер 512 байт для ответа IDENTIFY
        let id_buf_layout = core::alloc::Layout::from_size_align(512, 16).unwrap();
        let id_buf = alloc::alloc::alloc_zeroed(id_buf_layout);
        if id_buf.is_null() {
            alloc::alloc::dealloc(ct_ptr, ct_layout);
            return None;
        }
        let id_buf_phys = virt_to_phys(id_buf as u64);

        // FIS — IDENTIFY DEVICE (ATA_CMD_IDENTIFY = 0xEC)
        let fis = ct_ptr as *mut FisRegH2d;
        fis_command(&mut *fis, ATA_CMD_IDENTIFY);
        (*fis).device = 0;

        // PRD — указывает на буфер с ответом
        let prd = (ct_ptr as *mut u8).add(128) as *mut PrdEntry;
        (*prd).base_addr = id_buf_phys as u32;
        (*prd).base_addr_hi = (id_buf_phys >> 32) as u32;
        (*prd).byte_count = 512; // 512 байт, без прерывания

        // Command Header
        fill_cmd_header(cmd, false, ct_phys);

        // Запускаем команду
        port_write32(port_num, PORT_CI, 1);

        // Ждём завершения
        timeout = 2_000_000;
        while port_read32(port_num, PORT_CI) & 1 != 0 && timeout > 0 {
            timeout -= 1;
        }

        alloc::alloc::dealloc(ct_ptr, ct_layout);

        if timeout == 0 {
            // Диагностика: какие биты остались — покажет, дошла ли команда.
            let cmd_reg = port_read32(port_num, PORT_CMD);
            let tfd = port_read32(port_num, PORT_TFD);
            let ci = port_read32(port_num, PORT_CI);
            let serr = port_read32(port_num, PORT_SERR);
            println!(
                "[AHCI] Port {}: identify command timeout (CMD={:#x} TFD={:#x} CI={:#x} SERR={:#x})",
                port_num, cmd_reg, tfd, ci, serr
            );
            alloc::alloc::dealloc(id_buf, id_buf_layout);
            return None;
        }

        if let Some(is) = command_error(port_num) {
            println!("[AHCI] Port {}: identify error IS={:#x}", port_num, is);
            alloc::alloc::dealloc(id_buf, id_buf_layout);
            return None;
        }

        // Парсим данные IDENTIFY (256 слов по 16 бит)
        let data = core::slice::from_raw_parts(id_buf as *const u16, 256);

        // Секторов (LBA48) — слова 100..103
        let total_sectors = (data[100] as u64)
            | ((data[101] as u64) << 16)
            | ((data[102] as u64) << 32)
            | ((data[103] as u64) << 48);

        // Серийный номер — слова 10..19 (20 байт, ASCII с байт-свопом)
        let mut serial_raw = [0u8; 20];
        for i in 0..10 {
            let w = data[10 + i];
            serial_raw[i * 2] = (w >> 8) as u8;
            serial_raw[i * 2 + 1] = w as u8;
        }
        let serial_end = serial_raw.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
        let serial = String::from_utf8_lossy(&serial_raw[..serial_end]).into_owned();

        // Модель диска — слова 27..46 (40 байт)
        let mut model_raw = [0u8; 40];
        for i in 0..20 {
            let w = data[27 + i];
            model_raw[i * 2] = (w >> 8) as u8;
            model_raw[i * 2 + 1] = w as u8;
        }
        let model_end = model_raw.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
        let model = String::from_utf8_lossy(&model_raw[..model_end]).into_owned();

        // Размер сектора — по умолчанию 512
        let sector_size: u16 = if data.len() > 106 && data[106] & (1 << 12) != 0 {
            // Объявлен физический размер сектора
            let phys_sect_words = data[118] as u32;
            if phys_sect_words > 0 { (phys_sect_words * 2) as u16 } else { 512 }
        } else {
            512
        };

        alloc::alloc::dealloc(id_buf, id_buf_layout);

        println!("[AHCI] Port {}: {} — {}sect ({} bytes/sect)",
            port_num, model, total_sectors, sector_size);

        Some(DiskInfo { total_sectors, sector_size, serial, model })
    }
}
