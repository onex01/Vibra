// VirtIO Block Driver — базовый драйвер для QEMU VirtIO Block Device.
//
// VirtIO использует shared memory descriptors (VRing) для обмена данными
// между guest и host. Для QEMU: -device virtio-blk-device.
//
// Регистры (MMIO):
//   0x00: MagicValue (ro) = 0x74726976 ("virt")
//   0x04: Version (ro) = 2
//   0x08: DeviceID (ro) = 2 (block)
//   0x0c: VendorID (ro)
//   0x10: DeviceFeatures (ro)
//   0x14: DeviceFeaturesSel (wo)
//   0x20: DriverFeatures (wo)
//   0x24: DriverFeaturesSel (wo)
//   0x30: QueueSel (wo)
//   0x34: QueueSizeMax (ro)
//   0x44: QueueReady (rw)
//   0x50: QueueNotify (wo)
//   0x60: InterruptStatus (ro)
//   0x70: Status (rw)
//   0x80: QueueDescLow/High (wo)
//   0x90: QueueDriverLow/High (wo)
//   0xa0: QueueDeviceLow/High (wo)
//   0xfe: ConfigGeneration (ro)
//   0x100: Config (rw)

use alloc::vec::Vec;
use spin::Mutex;

const VIRTIO_MMIO_BASE: u64 = 0x0a000000; // QEMU default for virtio-mmio

const MAGIC: u32 = 0x74726976;
const VERSION: u32 = 2;
const DEVICE_ID_BLOCK: u32 = 2;

// Feature bits
const VIRTIO_BLK_F_SIZE_MAX: u32 = 1;
const VIRTIO_BLK_F_SEG_MAX: u32 = 2;
const VIRTIO_BLK_F_GEOMETRY: u32 = 4;
const VIRTIO_BLK_F_BLK_SIZE: u32 = 6;
const VIRTIO_BLK_F_FLUSH: u32 = 9;
const VIRTIO_BLK_F_TOPOLOGY: u32 = 10;

// Status bits
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
const VIRTIO_STATUS_FAILED: u8 = 128;

// Request types
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;

/// VRing Descriptor
#[repr(C, packed)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

/// VirtIO Block request header
#[repr(C, packed)]
struct VirtioBlkReq {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

/// VirtIO Block request status
#[repr(C, packed)]
struct VirtioBlkResp {
    status: u8,
}

const SECTOR_SIZE: usize = 512;

pub struct VirtioBlock {
    base: u64,
    queue_size: u16,
    status: u8,
    disk_size: u64,
    sector_count: u64,
    ready: bool,
}

impl VirtioBlock {
    pub fn new(base: u64) -> Self {
        Self {
            base,
            queue_size: 0,
            status: 0,
            disk_size: 0,
            sector_count: 0,
            ready: false,
        }
    }

    unsafe fn read32(&self, offset: u64) -> u32 {
        let ptr = (self.base + offset) as *const u32;
        core::ptr::read_volatile(ptr)
    }

    unsafe fn write32(&self, offset: u64, val: u32) {
        let ptr = (self.base + offset) as *mut u32;
        core::ptr::write_volatile(ptr, val);
    }

    unsafe fn read8(&self, offset: u64) -> u8 {
        let ptr = (self.base + offset) as *const u8;
        core::ptr::read_volatile(ptr)
    }

    unsafe fn write8(&self, offset: u64, val: u8) {
        let ptr = (self.base + offset) as *mut u8;
        core::ptr::write_volatile(ptr, val);
    }

    /// Попытка обнаружить и инициализировать устройство
    pub fn probe(&mut self) -> Result<(), &'static str> {
        unsafe {
            let magic = self.read32(0x00);
            if magic != MAGIC {
                return Err("not a VirtIO device (bad magic)");
            }

            let version = self.read32(0x04);
            if version != VERSION {
                return Err("unsupported VirtIO version");
            }

            let device_id = self.read32(0x08);
            if device_id != DEVICE_ID_BLOCK {
                return Err("not a block device");
            }

            // ACKNOWLEDGE + DRIVER
            self.status = VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER;
            self.write8(0x70, self.status);

            // Read device features
            self.write32(0x14, 0); // DeviceFeaturesSel = 0
            let features_lo = self.read32(0x10);

            // Negotiate features (disable everything complex for now)
            let driver_features: u32 = 0; // Accept no features for simplicity
            self.write32(0x24, 0); // DriverFeaturesSel = 0
            self.write32(0x20, driver_features);

            // DRIVER_OK
            self.status |= VIRTIO_STATUS_DRIVER_OK;
            self.write8(0x70, self.status);

            // Read config: capacity (sector count)
            // Config starts at offset 0x100
            let capacity_lo = self.read32(0x100);
            let capacity_hi = self.read32(0x104);
            self.sector_count = ((capacity_hi as u64) << 32) | (capacity_lo as u64);
            self.disk_size = self.sector_count * SECTOR_SIZE as u64;

            crate::println!("[VIRTIO-BLK] Found at {:#x}: {} sectors ({} MB)",
                self.base, self.sector_count, self.disk_size / (1024 * 1024));

            self.ready = true;
        }
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn disk_size(&self) -> u64 {
        self.disk_size
    }

    pub fn sector_count(&self) -> u64 {
        self.sector_count
    }
}

/// Глобальный экземпляр VirtIO Block
static VIRTIO_BLK: Mutex<Option<VirtioBlock>> = Mutex::new(None);

/// Попытка обнаружить VirtIO Block на стандартных MMIO адресах
pub fn probe_devices() {
    // QEMU virt board: virtio-mmio始于 0x0a000000
    let bases: [u64; 4] = [0x0a000000, 0x0a001000, 0x0a002000, 0x0a003000];

    for &base in &bases {
        unsafe {
            let magic = core::ptr::read_volatile(base as *const u32);
            if magic == MAGIC {
                let mut dev = VirtioBlock::new(base);
                match dev.probe() {
                    Ok(()) => {
                        *VIRTIO_BLK.lock() = Some(dev);
                        crate::println!("[VIRTIO-BLK] Device ready at {:#x}", base);
                        return;
                    }
                    Err(e) => {
                        crate::println!("[VIRTIO-BLK] Device at {:#x}: {}", base, e);
                    }
                }
            }
        }
    }
    crate::println!("[VIRTIO-BLK] No block devices found");
}

/// Прочитать сектор с диска (заглушка — требует VRing)
pub fn read_sector(_sector: u64, _buf: &mut [u8]) -> Result<(), &'static str> {
    let dev = VIRTIO_BLK.lock();
    if dev.is_none() {
        return Err("no VirtIO block device");
    }
    // TODO: implement VRing-based I/O
    Err("readSector not yet implemented (needs VRing)")
}

/// Записать сектор на диск (заглушка)
pub fn write_sector(_sector: u64, _buf: &[u8]) -> Result<(), &'static str> {
    let dev = VIRTIO_BLK.lock();
    if dev.is_none() {
        return Err("no VirtIO block device");
    }
    // TODO: implement VRing-based I/O
    Err("writeSector not yet implemented (needs VRing)")
}
