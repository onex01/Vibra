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

/// Command Header — описывает одну ATA команду
#[repr(C)]
struct CommandHeader {
    // DW0:.command
    cfl: u8,      // Command FIS Length (dw count, 0=2dw, 1=3dw, 2=4dw)
    pm: u8,       // Prefetchable Maximum
    _reserved1: u8,
    c: u8,        // Clear (busy after command)
    // DW0: features
    features: u8,
    // DW1
    lba_low: u8,
    lba_mid: u8,
    lba_high: u8,
    device: u8,
    // DW2
    features_exp: u8,
    lba_low_exp: u8,
    lba_mid_exp: u8,
    lba_high_exp: u8,
    // DW3
    sector_count: u16,
    // DW4
    _reserved2: u16,
    // DW5-6: PRD Table Physical Address (not used for simple reads)
    prdtl: u16,   // Physical Region Descriptor Table Length
    prdbc: u32,   // PRD Byte Count
    // DW7-8
    command_table_base: u64,
}

/// FIS Register H2D — Host to Device FIS
#[repr(C)]
struct FisRegH2d {
    fis_type: u8,     // FIS_TYPE_REG_H2D = 0x27
    pmport: u8,       // Port multiplier, 0
    _reserved0: u8,
    command: u8,      // ATA command
    features: u8,
    lba_low: u8,
    lba_mid: u8,
    lba_high: u8,
    device: u8,
    lba_low_exp: u8,
    lba_mid_exp: u8,
    lba_high_exp: u8,
    features_exp: u8,
    sector_count_lo: u8,
    sector_count_hi: u8,
    _reserved1: u8,
    control: u8,
    _reserved2: [u8; 4],
}

/// PRD Entry — Physical Region Descriptor
#[repr(C)]
struct PrdEntry {
    base_addr: u32,
    _reserved: u32,
    _reserved2: u32,
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

        // Enable interrupts
        hba_write32(HBA_GHC, hba_read32(HBA_GHC) | GHC_IE);

        // Сканируем порты
        let mut ports = AHCI_PORTS.lock();
        for i in 0..num_ports {
            if pi & (1 << i) == 0 { continue; }
            let i = i as u32;

            let ssts = port_read32(i, PORT_SSTS);
            let det = ssts & 0x0F; // Device Detection
            let ipm = (ssts >> 8) & 0x0F; // Interface Power Management

            if det == 0x03 && ipm == 0x01 {
                // Device connected and active
                let sig = port_read32(i, PORT_SIG);
                let sig_str = match sig >> 16 {
                    0x0000 => "ATAPI",
                    0xEB14 => "ATAPI",
                    0x9669 => "SEMB",
                    0xC33C => "SATA PM",
                    _ => "ATA/SATA",
                };

                println!("[AHCI] Port {}: {} ({:#010x})", i, sig_str, sig);

                // Инициализируем порт
                init_port(i);

                ports.push(AhciPort { num: i, active: true, ssts });
            } else {
                println!("[AHCI] Port {}: no device (DET={}, IPM={})", i, det, ipm);
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
        (*fis).fis_type = FIS_TYPE_REG_H2D;
        (*fis).command = if buffer.as_ptr() as u64 > hhdm {
            ATA_CMD_READ_DMA_EXT
        } else {
            ATA_CMD_READ_DMA_EXT
        };
        (*fis).device = (1 << 6) | ((lba >> 24) & 0x0F) as u8; // LBA mode, high bits
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
        (*prd).byte_count = (buffer.len() as u32) | (1 << 31); // interrupt on complete

        // Command Header
        cmd.cfl = 5; // FIS is 5 dwords (20 bytes)
        cmd.c = 1;   // Clear busy
        cmd.prdtl = 1; // 1 PRD entry
        cmd.prdbc = 0;
        cmd.command_table_base = ct_phys;

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
        let is = port_read32(port_num, PORT_IS);
        if is & 1 != 0 {
            println!("[AHCI] Port {}: error IS={:#x}", port_num, is);
            port_write32(port_num, PORT_IS, 0xFFFFFFFF);
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
        (*fis).fis_type = FIS_TYPE_REG_H2D;
        (*fis).command = ATA_CMD_WRITE_DMA_EXT;
        (*fis).device = (1 << 6) | ((lba >> 24) & 0x0F) as u8;
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
        (*prd).byte_count = (buffer.len() as u32) | (1 << 31);

        // Command Header
        cmd.cfl = 5;
        cmd.c = 1;
        cmd.prdtl = 1;
        cmd.prdbc = 0;
        cmd.command_table_base = ct_phys;

        // Command Issue
        port_write32(port_num, PORT_CI, 1);

        // Wait
        timeout = 5_000_000;
        while port_read32(port_num, PORT_CI) & 1 != 0 && timeout > 0 {
            timeout -= 1;
        }

        alloc::alloc::dealloc(ct_ptr, ct_layout);

        if timeout == 0 { return false; }
        let is = port_read32(port_num, PORT_IS);
        if is & 1 != 0 {
            port_write32(port_num, PORT_IS, 0xFFFFFFFF);
            return false;
        }
        true
    }
}
