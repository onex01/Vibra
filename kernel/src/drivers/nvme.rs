// NVMe (Non-Volatile Memory Express) — драйвер SSD дисков.
//
// NVMe — высокопроизводительный интерфейс для SSD накопителей.
// Использует PCI class 0x01, subclass 0x08.
// BAR0 содержит MMIO регистры контроллера.
// Команды отправляются через Admin Queue и I/O Queue.
//
// Регистры (смещения от BAR0):
//   CAP  (0x00) — Capabilities
//   VS   (0x08) — Version
//   CC   (0x14) — Controller Configuration
//   CSTS (0x1C) — Controller Status
//   AQA  (0x24) — Admin Queue Attributes
//   ASQ  (0x28) — Admin Submission Queue Base Address
//   ACQ  (0x30) — Admin Completion Queue Base Address

use crate::println;
use spin::Mutex;
use core::sync::atomic::{AtomicU64, Ordering};

// NVMe регистры (смещения от BAR0)
const NVME_REG_CAP: u64 = 0x00;
const NVME_REG_VS: u64 = 0x08;
const NVME_REG_CC: u64 = 0x14;
const NVME_REG_CSTS: u64 = 0x1C;
const NVME_REG_AQA: u64 = 0x24;
const NVME_REG_ASQ: u64 = 0x28;
const NVME_REG_ACQ: u64 = 0x30;

// CC (Controller Configuration) биты
const CC_EN: u32 = 1;           // Enable
const CC_CSS_NVM: u32 = 0 << 4; // Command Set Selected: NVM
const CC_MPS_4K: u32 = 0 << 7;  // Memory Page Size: 4K (0 = 4K)
const CC_AMS_RR: u32 = 0 << 11; // Arbitration: Round Robin
const CC_SHN_NONE: u32 = 0 << 14; // Shutdown Notification: none
const CC_IOSQES: u32 = 6 << 16;  // I/O Queue Entry Size: 2^6 = 64
const CC_IOCQES: u32 = 4 << 20;  // I/O Completion Queue Entry Size: 2^4 = 16

// CSTS (Controller Status) биты
const CSTS_RDY: u32 = 1;       // Ready
const CSTS_CFS: u32 = 1 << 1;  // Controller Fatal Status
const CSTS_SHST: u32 = 3 << 2; // Shutdown Status

// NVMe Submission Queue Entry (64 байта)
#[repr(C, packed)]
struct NvmeSubQueueEntry {
    opcode: u8,
    flags: u8,
    cid: u16,
    nsid: u32,
    _reserved: u64,
    mptr: u64,
    prp1: u64,
    prp2: u64,
    _reserved2: [u32; 9],
}

// NVMe Completion Queue Entry (16 байт)
#[repr(C, packed)]
struct NvmeCompQueueEntry {
    dw0: u32,     // Command Specific
    dw1: u32,     // Reserved
    sq_head: u16,
    sq_id: u16,
    cid: u16,
    status: u16,  // Phase + Status
}

// NVMe Admin命令 opcodes
const NVME_ADMIN_IDENTIFY: u8 = 0x06;
const NVME_ADMIN_CREATE_IO_SQ: u8 = 0x01;
const NVME_ADMIN_CREATE_IO_CQ: u8 = 0x05;

// NVMe I/O命令 opcodes
const NVME_IO_READ: u8 = 0x02;
const NVME_IO_WRITE: u8 = 0x01;

// MMIO базовый адрес
static NVME_BASE: AtomicU64 = AtomicU64::new(0);

#[inline]
unsafe fn nvme_read32(offset: u64) -> u32 {
    let base = NVME_BASE.load(Ordering::Relaxed);
    core::ptr::read_volatile((base + offset) as *const u32)
}

#[inline]
unsafe fn nvme_write32(offset: u64, val: u32) {
    let base = NVME_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

#[inline]
unsafe fn nvme_read64(offset: u64) -> u64 {
    let base = NVME_BASE.load(Ordering::Relaxed);
    core::ptr::read_volatile((base + offset) as *const u64)
}

#[inline]
unsafe fn nvme_write64(offset: u64, val: u64) {
    let base = NVME_BASE.load(Ordering::Relaxed);
    core::ptr::write_volatile((base + offset) as *mut u64, val);
}

/// Физический адрес виртуального указателя (через HHDM)
fn virt_to_phys(virt: u64) -> u64 {
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    virt - hhdm
}

/// NVMe контроллер
pub struct NvmeController {
    mmio_base: u64,
    doorbell_stride: u64,
    page_size: usize,
    // Admin queues
    admin_sq: *mut NvmeSubQueueEntry,
    admin_cq: *mut NvmeCompQueueEntry,
    admin_sq_phys: u64,
    admin_cq_phys: u64,
    admin_sq_tail: u16,
    admin_cq_head: u16,
    admin_phase: bool,
    // I/O queues
    io_sq: *mut NvmeSubQueueEntry,
    io_cq: *mut NvmeCompQueueEntry,
    io_sq_phys: u64,
    io_cq_phys: u64,
    io_sq_tail: u16,
    io_cq_head: u16,
    io_phase: bool,
    // Информация о диске
    block_size: u32,
    total_blocks: u64,
    ns_id: u32,
    // Буфер для идентификации
    id_buf: *mut u8,
    id_buf_phys: u64,
}

unsafe impl Send for NvmeController {}
unsafe impl Sync for NvmeController {}

impl NvmeController {
    /// Отправить команду в Admin Queue и дождаться ответа
    unsafe fn admin_submit(&mut self, cmd: &NvmeSubQueueEntry) -> Option<NvmeCompQueueEntry> {
        let slot = self.admin_sq_tail as usize;
        core::ptr::copy_nonoverlapping(
            cmd as *const _ as *const u8,
            self.admin_sq.add(slot) as *mut u8,
            core::mem::size_of::<NvmeSubQueueEntry>(),
        );

        self.admin_sq_tail = (self.admin_sq_tail + 1) % 64;
        let base = self.mmio_base;

        // Дверной звонок — записываем new tail в Admin SQ y_doorbell
        let tail_offset = 0x1000; // Admin SQ y_tail doorbell
        core::ptr::write_volatile((base + tail_offset) as *mut u32, self.admin_sq_tail as u32);

        // Ждём completion
        let mut timeout = 5_000_000u32;
        while timeout > 0 {
            let comp = &*self.admin_cq.add(self.admin_cq_head as usize);
            if (comp.status >> 0) & 1 == self.admin_phase as u16 {
                // Фаза совпала — команда завершена
                let result = core::ptr::read(comp as *const _ as *const NvmeCompQueueEntry);
                self.admin_cq_head = (self.admin_cq_head + 1) % 64;
                if self.admin_cq_head == 0 {
                    self.admin_phase = !self.admin_phase;
                }

                // Дверной звонок — записываем new head в Admin CQ y_doorbell
                let head_offset = 0x1000 + 4;
                core::ptr::write_volatile((base + head_offset) as *mut u32, self.admin_cq_head as u32);

                return Some(result);
            }
            timeout -= 1;
        }

        println!("[NVMe] Тайм-аут admin команды (opcode={:#x})", cmd.opcode);
        None
    }

    /// Отправить I/O команду
    unsafe fn io_submit(&mut self, cmd: &NvmeSubQueueEntry) -> Option<NvmeCompQueueEntry> {
        let slot = self.io_sq_tail as usize;
        core::ptr::copy_nonoverlapping(
            cmd as *const _ as *const u8,
            self.io_sq.add(slot) as *mut u8,
            core::mem::size_of::<NvmeSubQueueEntry>(),
        );

        self.io_sq_tail = (self.io_sq_tail + 1) % 64;
        let base = self.mmio_base;

        // Дверной звонок — I/O SQ y_tail
        let tail_offset = 0x1000 + 8;
        core::ptr::write_volatile((base + tail_offset) as *mut u32, self.io_sq_tail as u32);

        // Ждём completion
        let mut timeout = 5_000_000u32;
        while timeout > 0 {
            let comp = &*self.io_cq.add(self.io_cq_head as usize);
            if (comp.status >> 0) & 1 == self.io_phase as u16 {
                let result = core::ptr::read(comp as *const _ as *const NvmeCompQueueEntry);
                self.io_cq_head = (self.io_cq_head + 1) % 64;
                if self.io_cq_head == 0 {
                    self.io_phase = !self.io_phase;
                }

                // Дверной звонок — I/O CQ y_head
                let head_offset = 0x1000 + 12;
                core::ptr::write_volatile((base + head_offset) as *mut u32, self.io_cq_head as u32);

                return Some(result);
            }
            timeout -= 1;
        }

        println!("[NVMe] Тайм-аут I/O команды (opcode={:#x})", cmd.opcode);
        None
    }

    /// Выполнить команду Identify Controller
    unsafe fn identify_controller(&mut self) {
        // Буфер 4096 байт для ответа Identify
        let buf_layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();
        let buf = alloc::alloc::alloc_zeroed(buf_layout);
        if buf.is_null() {
            println!("[NVMe] Ошибка выделения памяти для Identify");
            return;
        }
        self.id_buf = buf;
        self.id_buf_phys = virt_to_phys(buf as u64);

        let mut cmd = NvmeSubQueueEntry {
            opcode: NVME_ADMIN_IDENTIFY,
            flags: 0,
            cid: 1,
            nsid: 0,
            _reserved: 0,
            mptr: 0,
            prp1: self.id_buf_phys,
            prp2: 0,
            _reserved2: [0; 9],
        };

        // CNDW10: CSType=1 (Controller), CNS=0
        // Encode CSType into the first dword of _reserved2
        // Actually, for Identify Controller: CNTID=0 in DW10 bits[15:0], CSTYPE=1 in DW10 bits[31:16]
        // DW10: CSTYPE (bits 31:16) = 1, CNTID (bits 15:0) = 0
        let dw10: u32 = 0x0001_0000; // CSTYPE=1 (Controller Identify)
        cmd._reserved2[0] = dw10;

        match self.admin_submit(&cmd) {
            Some(comp) => {
                let sc = (comp.status >> 1) & 0x7F;
                if sc == 0 {
                    // Парсим данные Identify Controller
                    let data = core::slice::from_raw_parts(self.id_buf as *const u16, 2048);

                    // Модель — слова 13..23 (20 байт, ASCII с байт-свопом)
                    let mut model_raw = [0u8; 40];
                    for i in 0..20 {
                        let w = data[13 + i];
                        model_raw[i * 2] = (w >> 8) as u8;
                        model_raw[i * 2 + 1] = w as u8;
                    }
                    let model_end = model_raw.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
                    let model = alloc::string::String::from_utf8_lossy(&model_raw[..model_end]);

                    // серийный номер — слова 3..7 (8 байт, ASCII с байт-свопом)
                    let mut serial_raw = [0u8; 16];
                    for i in 0..4 {
                        let w = data[3 + i];
                        serial_raw[i * 2] = (w >> 8) as u8;
                        serial_raw[i * 2 + 1] = w as u8;
                    }
                    let serial_end = serial_raw.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
                    let serial = alloc::string::String::from_utf8_lossy(&serial_raw[..serial_end]);

                    println!("[NVMe] Контроллер: {}", model);
                    println!("[NVMe] Серийный: {}", serial);
                    println!("[NVMe] Макс. عدد команд в очереди: {}", data[512] as u32 + 1);
                } else {
                    println!("[NVMe] Identify ошибка: SC={:#x}", sc);
                }
            }
            None => {
                println!("[NVMe] Identify не удалась");
            }
        }
    }

    /// Инициализировать I/O queues
    unsafe fn create_io_queues(&mut self) -> bool {
        // Выделяем память для I/O queues (по 64 записи каждая)
        let sq_layout = core::alloc::Layout::from_size_align(
            core::mem::size_of::<NvmeSubQueueEntry>() * 64, 4096
        ).unwrap();
        let cq_layout = core::alloc::Layout::from_size_align(
            core::mem::size_of::<NvmeCompQueueEntry>() * 64, 4096
        ).unwrap();

        self.io_sq = alloc::alloc::alloc_zeroed(sq_layout) as *mut NvmeSubQueueEntry;
        self.io_cq = alloc::alloc::alloc_zeroed(cq_layout) as *mut NvmeCompQueueEntry;

        if self.io_sq.is_null() || self.io_cq.is_null() {
            println!("[NVMe] Ошибка выделения памяти для I/O queues");
            return false;
        }

        self.io_sq_phys = virt_to_phys(self.io_sq as u64);
        self.io_cq_phys = virt_to_phys(self.io_cq as u64);

        // Создаём I/O Completion Queue (opcode 0x05)
        let mut cq_cmd = NvmeSubQueueEntry {
            opcode: NVME_ADMIN_CREATE_IO_CQ,
            flags: 0,
            cid: 2,
            nsid: 0,
            _reserved: 0,
            mptr: 0,
            prp1: self.io_cq_phys,
            prp2: 0,
            _reserved2: [0; 9],
        };
        // DW10: QID=0, QSIZE=63 (64 entries - 1)
        cq_cmd._reserved2[0] = (63 << 16) | 0; // QSIZE=63, QID=0
        // DW11: IV=0, IEN=1 (Interrupts Enabled), PC=1 (Physically Contiguous)
        cq_cmd._reserved2[1] = 0x00000003; // PC=1, IEN=1

        match self.admin_submit(&cq_cmd) {
            Some(comp) => {
                let sc = (comp.status >> 1) & 0x7F;
                if sc != 0 {
                    println!("[NVMe] Ошибка создания I/O CQ: SC={:#x}", sc);
                    return false;
                }
            }
            None => return false,
        }

        // Создаём I/O Submission Queue (opcode 0x01)
        let mut sq_cmd = NvmeSubQueueEntry {
            opcode: NVME_ADMIN_CREATE_IO_SQ,
            flags: 0,
            cid: 3,
            nsid: 0,
            _reserved: 0,
            mptr: 0,
            prp1: self.io_sq_phys,
            prp2: 0,
            _reserved2: [0; 9],
        };
        // DW10: QSIZE=63, QID=0
        sq_cmd._reserved2[0] = (63 << 16) | 0; // QSIZE=63, QID=0
        // DW11: CQID=0 (associated CQ), PC=1, QPRIO=0
        sq_cmd._reserved2[1] = 0x00000001; // PC=1, CQID=0

        match self.admin_submit(&sq_cmd) {
            Some(comp) => {
                let sc = (comp.status >> 1) & 0x7F;
                if sc != 0 {
                    println!("[NVMe] Ошибка создания I/O SQ: SC={:#x}", sc);
                    return false;
                }
            }
            None => return false,
        }

        println!("[NVMe] I/O очереди созданы (SQ + CQ)");
        true
    }

    /// Идентифицировать namespace
    unsafe fn identify_namespace(&mut self) {
        let buf_layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();
        let buf = alloc::alloc::alloc_zeroed(buf_layout);
        if buf.is_null() { return; }
        let buf_phys = virt_to_phys(buf as u64);

        let mut cmd = NvmeSubQueueEntry {
            opcode: NVME_ADMIN_IDENTIFY,
            flags: 0,
            cid: 4,
            nsid: 1, // Namespace 1
            _reserved: 0,
            mptr: 0,
            prp1: buf_phys,
            prp2: 0,
            _reserved2: [0; 9],
        };
        // DW10: CSTYPE=0 (Namespace Identify), CNS=0
        cmd._reserved2[0] = 0x00000000;

        match self.admin_submit(&cmd) {
            Some(comp) => {
                let sc = (comp.status >> 1) & 0x7F;
                if sc == 0 {
                    let data = core::slice::from_raw_parts(buf as *const u8, 4096);

                    // NSZE (Total Size) — байты 0..7
                    self.total_blocks = core::ptr::read(data.as_ptr() as *const u64);
                    // NLBA (Number of Logical Blocks) — тоже байты 0..7

                    // LBA Format (bytes 126-127): FLBAS
                    let flbas = data[26] as usize;
                    let lba_fmt_index = flbas & 0x0F;

                    // LBA Format support descriptors начиная с байта 128
                    // Каждый descriptor: 4 байта (MS, LBADS, RP, etc.)
                    let fmt_offset = 128 + lba_fmt_index * 4;
                    if fmt_offset + 1 < data.len() {
                        let lbads = data[fmt_offset + 1]; // LBA Data Size
                        self.block_size = 1u32 << lbads;
                    } else {
                        self.block_size = 512; // По умолчанию
                    }

                    println!("[NVMe] Namespace 1: {} блоков, {} байт/блок",
                        self.total_blocks, self.block_size);
                } else {
                    println!("[NVMe] Identify NS ошибка: SC={:#x}", sc);
                    self.block_size = 512;
                    self.total_blocks = 0;
                }
            }
            None => {
                self.block_size = 512;
                self.total_blocks = 0;
            }
        }

        alloc::alloc::dealloc(buf, buf_layout);
    }
}

/// Глобальный NVMe контроллер
static NVME_CONTROLLER: Mutex<Option<NvmeController>> = Mutex::new(None);

/// Инициализация NVMe драйвера
pub fn init() {
    println!("[NVMe] Поиск NVMe контроллера...");

    // Ищем NVMe через PCI (class 0x01, subclass 0x08)
    let nvme_device = match super::pci::find_device(0x01, 0x08) {
        Some(d) => d,
        None => {
            println!("[NVMe] NVMe контроллер не найден");
            return;
        }
    };

    println!("[NVMe] Найден контроллер PCI [{:02X}:{:02X}.{}]",
        nvme_device.bus, nvme_device.dev, nvme_device.func);
    println!("[NVMe]   Vendor: {:04X} Device: {:04X}",
        nvme_device.vendor_id, nvme_device.device_id);

    // Включаем Bus Master и Memory Space
    unsafe {
        super::pci::enable_bus_master(nvme_device.bus, nvme_device.dev, nvme_device.func);
    }

    // BAR0 — MMIO регистры NVMe
    let bar0 = nvme_device.bar0 & 0xFFFFFFF0;
    if bar0 == 0 {
        println!("[NVMe] ОШИБКА: BAR0 = 0 — контроллер не сконфигурирован");
        return;
    }

    // MMIO через HHDM
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed);
    let mmio_base = hhdm + bar0 as u64;
    NVME_BASE.store(mmio_base, Ordering::SeqCst);

    println!("[NVMe] BAR0: физ={:#x} вирт={:#x}", bar0, mmio_base);

    unsafe {
        // Читаем CAP для определения doorbell stride
        let cap = nvme_read64(NVME_REG_CAP);
        let doorbell_stride = 4u64 << ((cap >> 32) & 0xF); // CAP.DSTRD
        println!("[NVMe] CAP: doorbell stride = {}", doorbell_stride);

        let mpsmin = ((cap >> 48) & 0xF) as u32; // CAP.MPSMIN
        let page_size = 1u32 << (12 + mpsmin);
        println!("[NVMe] CAP: размер страницы = {} байт", page_size);

        // Сброс контроллера — обнуляем CC
        nvme_write32(NVME_REG_CC, 0);

        // Ждём пока CSTS.RDY станет 0
        let mut timeout = 5_000_000u32;
        while nvme_read32(NVME_REG_CSTS) & CSTS_RDY != 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 {
            println!("[NVMe] Предупреждение: контроллер не сбросился (CSTS.RDY=1)");
        }

        println!("[NVMe] Контроллер сброшен, начинаем инициализацию");

        // Выделяем Admin queues (64 записи каждая)
        let sq_layout = core::alloc::Layout::from_size_align(
            core::mem::size_of::<NvmeSubQueueEntry>() * 64, 4096
        ).unwrap();
        let cq_layout = core::alloc::Layout::from_size_align(
            core::mem::size_of::<NvmeCompQueueEntry>() * 64, 4096
        ).unwrap();

        let admin_sq = alloc::alloc::alloc_zeroed(sq_layout) as *mut NvmeSubQueueEntry;
        let admin_cq = alloc::alloc::alloc_zeroed(cq_layout) as *mut NvmeCompQueueEntry;

        if admin_sq.is_null() || admin_cq.is_null() {
            println!("[NVMe] ОШИБКА: Не удалось выделить память для Admin queues");
            return;
        }

        let admin_sq_phys = virt_to_phys(admin_sq as u64);
        let admin_cq_phys = virt_to_phys(admin_cq as u64);

        println!("[NVMe] Admin SQ: вирт={:#x} физ={:#x}", admin_sq as u64, admin_sq_phys);
        println!("[NVMe] Admin CQ: вирт={:#x} физ={:#x}", admin_cq as u64, admin_cq_phys);

        // Устанавливаем AQA (Admin Queue Attributes)
        // AQA: ACQS (bits 27:16) = 63, ASQS (bits 11:0) = 63
        nvme_write32(NVME_REG_AQA, (63 << 16) | 63);

        // Устанавливаем ASQ (Admin Submission Queue Base Address)
        nvme_write64(NVME_REG_ASQ, admin_sq_phys);

        // Устанавливам ACQ (Admin Completion Queue Base Address)
        nvme_write64(NVME_REG_ACQ, admin_cq_phys);

        // Включаем контроллер — CC.EN=1
        let cc = CC_EN
            | CC_CSS_NVM
            | CC_MPS_4K
            | CC_AMS_RR
            | CC_SHN_NONE
            | CC_IOSQES
            | CC_IOCQES;
        nvme_write32(NVME_REG_CC, cc);

        println!("[NVMe] CC записан: {:#x}, ожидаем CSTS.RDY...", cc);

        // Ждём пока CSTS.RDY станет 1
        timeout = 5_000_000;
        while nvme_read32(NVME_REG_CSTS) & CSTS_RDY == 0 && timeout > 0 {
            timeout -= 1;
        }
        if timeout == 0 {
            println!("[NVMe] ОШИБКА: Тайм-аут ожидания CSTS.RDY");
            println!("[NVMe]   CSTS = {:#x}", nvme_read32(NVME_REG_CSTS));
            return;
        }

        println!("[NVMe] Контроллер готов (CSTS.RDY=1)");

        let mut controller = NvmeController {
            mmio_base,
            doorbell_stride,
            page_size: page_size as usize,
            admin_sq,
            admin_cq,
            admin_sq_phys,
            admin_cq_phys,
            admin_sq_tail: 0,
            admin_cq_head: 0,
            admin_phase: true,
            io_sq: core::ptr::null_mut(),
            io_cq: core::ptr::null_mut(),
            io_sq_phys: 0,
            io_cq_phys: 0,
            io_sq_tail: 0,
            io_cq_head: 0,
            io_phase: true,
            block_size: 512,
            total_blocks: 0,
            ns_id: 1,
            id_buf: core::ptr::null_mut(),
            id_buf_phys: 0,
        };

        // Identify Controller
        controller.identify_controller();

        // Создаём I/O очереди
        if !controller.create_io_queues() {
            println!("[NVMe] ОШИБКА: Не удалось создать I/O очереди");
            return;
        }

        // Identify Namespace
        controller.identify_namespace();

        println!("[NVMe] NVMe драйвер инициализирован успешно");

        *NVME_CONTROLLER.lock() = Some(controller);
    }
}

/// Реализация DiskIo для NVMe
impl crate::fs::disk::DiskIo for NvmeController {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        let blocks_to_read = (buf.len() / self.block_size as usize) as u32;
        if blocks_to_read == 0 {
            return Ok(());
        }

        // Выделяем промежуточный DMA буфер
        let buf_size = blocks_to_read as usize * self.block_size as usize;
        let dma_layout = core::alloc::Layout::from_size_align(buf_size, 4096).unwrap();
        let dma_buf = unsafe { alloc::alloc::alloc_zeroed(dma_layout) };
        if dma_buf.is_null() {
            return Err(crate::fs::vfs::FsError::IoError);
        }
        let dma_phys = virt_to_phys(dma_buf as u64);

        unsafe {
            let mut cmd = NvmeSubQueueEntry {
                opcode: NVME_IO_READ,
                flags: 0,
                cid: 10,
                nsid: self.ns_id,
                _reserved: 0,
                mptr: 0,
                prp1: dma_phys,
                prp2: 0,
                _reserved2: [0; 9],
            };

            // DW10: SLBA (low 32 bits)
            cmd._reserved2[0] = sector as u32;
            // DW11: SLBA (high 32 bits)
            cmd._reserved2[1] = (sector >> 32) as u32;
            // DW12: Number of Logical Blocks (low 16 bits)
            cmd._reserved2[2] = (blocks_to_read - 1) as u32;

            match self.io_submit(&cmd) {
                Some(comp) => {
                    let sc = (comp.status >> 1) & 0x7F;
                    if sc == 0 {
                        core::ptr::copy_nonoverlapping(dma_buf, buf.as_mut_ptr(), buf_size);
                        alloc::alloc::dealloc(dma_buf, dma_layout);
                        Ok(())
                    } else {
                        alloc::alloc::dealloc(dma_buf, dma_layout);
                        Err(crate::fs::vfs::FsError::IoError)
                    }
                }
                None => {
                    alloc::alloc::dealloc(dma_buf, dma_layout);
                    Err(crate::fs::vfs::FsError::IoError)
                }
            }
        }
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), crate::fs::vfs::FsError> {
        let blocks_to_write = (buf.len() / self.block_size as usize) as u32;
        if blocks_to_write == 0 {
            return Ok(());
        }

        let buf_size = blocks_to_write as usize * self.block_size as usize;
        let dma_layout = core::alloc::Layout::from_size_align(buf_size, 4096).unwrap();
        let dma_buf = unsafe { alloc::alloc::alloc_zeroed(dma_layout) };
        if dma_buf.is_null() {
            return Err(crate::fs::vfs::FsError::IoError);
        }
        let dma_phys = virt_to_phys(dma_buf as u64);

        unsafe {
            // Копируем данные в DMA буфер
            core::ptr::copy_nonoverlapping(buf.as_ptr(), dma_buf, buf_size);

            let mut cmd = NvmeSubQueueEntry {
                opcode: NVME_IO_WRITE,
                flags: 0,
                cid: 11,
                nsid: self.ns_id,
                _reserved: 0,
                mptr: 0,
                prp1: dma_phys,
                prp2: 0,
                _reserved2: [0; 9],
            };

            cmd._reserved2[0] = sector as u32;
            cmd._reserved2[1] = (sector >> 32) as u32;
            cmd._reserved2[2] = (blocks_to_write - 1) as u32;

            match self.io_submit(&cmd) {
                Some(comp) => {
                    let sc = (comp.status >> 1) & 0x7F;
                    alloc::alloc::dealloc(dma_buf, dma_layout);
                    if sc == 0 {
                        Ok(())
                    } else {
                        Err(crate::fs::vfs::FsError::IoError)
                    }
                }
                None => {
                    alloc::alloc::dealloc(dma_buf, dma_layout);
                    Err(crate::fs::vfs::FsError::IoError)
                }
            }
        }
    }
}

/// Создать NVMe DiskIo объект для подключения к файловой системе
pub fn create_disk() -> Option<crate::alloc::boxed::Box<dyn crate::fs::disk::DiskIo>> {
    let ctrl = NVME_CONTROLLER.lock();
    if ctrl.is_none() {
        return None;
    }
    // Пока возвращаем None — нужен ownership transfer
    // В реальной системе контроллер должен быть передан в FS
    None
}

/// Проверить, доступен ли NVMe контроллер
pub fn is_available() -> bool {
    NVME_CONTROLLER.lock().is_some()
}

/// Получить информацию о диске
pub fn disk_info() -> Option<(u64, u32)> {
    let ctrl = NVME_CONTROLLER.lock();
    ctrl.as_ref().map(|c| (c.total_blocks, c.block_size))
}
