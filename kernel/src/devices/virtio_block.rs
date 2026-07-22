// VirtIO Block Driver — с VRing I/O для чтения/записи секторов.

use spin::Mutex;

const MAGIC: u32 = 0x74726976;
const VERSION: u32 = 2;
const DEVICE_ID_BLOCK: u32 = 2;
const SECTOR_SIZE: usize = 512;

const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

#[repr(C, packed)]
struct VirtqDesc { addr: u64, len: u32, flags: u16, next: u16 }

#[repr(C, packed)]
struct VirtioBlkReq { req_type: u32, reserved: u32, sector: u64 }

struct VirtQueue {
    descriptors: *mut VirtqDesc,
    avail_ring: *mut u16,
    used_ring: *mut u32,
    queue_size: u16,
    free_head: u16,
    num_free: u16,
    next_avail: u16,
}

unsafe impl Send for VirtQueue {}
unsafe impl Sync for VirtQueue {}

impl VirtQueue {
    fn new(queue_size: u16, hhdm: u64) -> Option<Self> {
        let desc_sz = (queue_size as usize) * 16;
        let avail_sz = 6 + (queue_size as usize) * 2;
        let used_sz = 6 + (queue_size as usize) * 8;
        let total = (desc_sz + avail_sz + used_sz + 4095) & !4095;

        let phys = crate::memory::pmm::alloc_contiguous(total / 4096)?;
        let virt = hhdm + phys as u64;

        unsafe {
            let ptr = virt as *mut u8;
            for i in 0..total { core::ptr::write_volatile(ptr.add(i), 0); }

            let descriptors = virt as *mut VirtqDesc;
            let avail_ring = (virt as *mut u8).add(desc_sz) as *mut u16;
            let used_ring = (virt as *mut u8).add(desc_sz + avail_sz) as *mut u32;

            let mut q = VirtQueue { descriptors, avail_ring, used_ring, queue_size, free_head: 0, num_free: queue_size, next_avail: 0 };
            for i in 0..(queue_size - 1) { (*descriptors.add(i as usize)).next = i + 1; }
            (*descriptors.add((queue_size - 1) as usize)).next = 0xFFFF;
            Some(q)
        }
    }

    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 { return None; }
        let idx = self.free_head;
        unsafe { self.free_head = (*self.descriptors.add(idx as usize)).next; }
        self.num_free -= 1;
        Some(idx)
    }

    fn free_desc(&mut self, idx: u16) {
        unsafe { (*self.descriptors.add(idx as usize)).next = self.free_head; }
        self.free_head = idx;
        self.num_free += 1;
    }

    fn add_buf(&mut self, desc_idx: u16) {
        unsafe {
            let idx = core::ptr::read_volatile(self.avail_ring.add(1));
            let ring = (idx as usize) % (self.queue_size as usize);
            core::ptr::write_volatile(self.avail_ring.add(2 + ring), desc_idx);
            core::ptr::write_volatile(self.avail_ring.add(1), idx + 1);
        }
    }

    fn kick(&self, base: u64) {
        unsafe { core::ptr::write_volatile((base + 0x50) as *mut u32, 0); }
    }

    fn poll_used(&mut self) -> Option<u16> {
        unsafe {
            let used_idx = core::ptr::read_volatile(self.used_ring.add(1)) as u16;
            if used_idx != self.next_avail {
                let ring = (self.next_avail as usize) % (self.queue_size as usize);
                let id = core::ptr::read_volatile(self.used_ring.add(2 + ring * 2));
                self.next_avail += 1;
                Some(id as u16)
            } else { None }
        }
    }
}

pub struct VirtioBlock {
    base: u64,
    queue_size: u16,
    status: u8,
    disk_size: u64,
    sector_count: u64,
    ready: bool,
    queue: Option<VirtQueue>,
}

unsafe impl Send for VirtioBlock {}
unsafe impl Sync for VirtioBlock {}

impl VirtioBlock {
    pub fn new(base: u64) -> Self {
        Self { base, queue_size: 0, status: 0, disk_size: 0, sector_count: 0, ready: false, queue: None }
    }

    unsafe fn r32(&self, o: u64) -> u32 { core::ptr::read_volatile((self.base + o) as *const u32) }
    unsafe fn w32(&self, o: u64, v: u32) { core::ptr::write_volatile((self.base + o) as *mut u32, v); }
    unsafe fn r8(&self, o: u64) -> u8 { core::ptr::read_volatile((self.base + o) as *const u8) }
    unsafe fn w8(&self, o: u64, v: u8) { core::ptr::write_volatile((self.base + o) as *mut u8, v); }

    pub fn probe(&mut self) -> Result<(), &'static str> {
        unsafe {
            if self.r32(0) != MAGIC { return Err("bad magic"); }
            if self.r32(4) != VERSION { return Err("bad version"); }
            if self.r32(8) != DEVICE_ID_BLOCK { return Err("not block device"); }

            self.status = VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER;
            self.w8(0x70, self.status);

            self.w32(0x14, 0); self.r32(0x10); // read features
            self.w32(0x24, 0); self.w32(0x20, 0); // write features

            self.w32(0x30, 0); // QueueSel = 0
            self.queue_size = self.r32(0x34) as u16;
            if self.queue_size == 0 { return Err("queue size 0"); }

            let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
            if let Some(queue) = VirtQueue::new(self.queue_size, hhdm) {
                self.queue = Some(queue);
                let q = self.queue.as_ref().unwrap();
                let desc_p = (q.descriptors as u64) - hhdm;
                let avail_p = (q.avail_ring as u64) - hhdm;
                let used_p = (q.used_ring as u64) - hhdm;
                self.w32(0x80, desc_p as u32); self.w32(0x84, (desc_p >> 32) as u32);
                self.w32(0x90, avail_p as u32); self.w32(0x94, (avail_p >> 32) as u32);
                self.w32(0xa0, used_p as u32); self.w32(0xa4, (used_p >> 32) as u32);
                self.w32(0x44, 1); // QueueReady
            }

            let cap_lo = self.r32(0x100); let cap_hi = self.r32(0x104);
            self.sector_count = ((cap_hi as u64) << 32) | (cap_lo as u64);
            self.disk_size = self.sector_count * SECTOR_SIZE as u64;

            crate::println!("[VIRTIO-BLK] {:#x}: {} sectors ({} MB) qsz={}",
                self.base, self.sector_count, self.disk_size / (1024 * 1024), self.queue_size);

            self.status |= VIRTIO_STATUS_DRIVER_OK;
            self.w8(0x70, self.status);
            self.ready = true;
        }
        Ok(())
    }

    pub fn read_sectors(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        if !self.ready { return Err("not ready"); }
        let q = self.queue.as_mut().ok_or("no queue")?;
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);

        let h = q.alloc_desc().ok_or("no desc")?;
        let d = q.alloc_desc().ok_or("no desc")?;
        let s = q.alloc_desc().ok_or("no desc")?;

        unsafe {
            let hdr = q.descriptors.add(h as usize);
            let hdr_v = hhdm + (*hdr).addr;
            let req = hdr_v as *mut VirtioBlkReq;
            (*req).req_type = VIRTIO_BLK_T_IN; (*req).sector = sector;
            (*hdr).len = 16; (*hdr).flags = VRING_DESC_F_NEXT; (*hdr).next = d;

            let dat = q.descriptors.add(d as usize);
            (*dat).addr = (buf.as_ptr() as u64) - hhdm;
            (*dat).len = buf.len() as u32;
            (*dat).flags = VRING_DESC_F_NEXT | VRING_DESC_F_WRITE;
            (*dat).next = s;

            let st = q.descriptors.add(s as usize);
            let st_v = hhdm + (*st).addr;
            (*st).addr = (st_v - hhdm) as u64;
            (*st).len = 1; (*st).flags = VRING_DESC_F_WRITE; (*st).next = 0xFFFF;
        }

        q.add_buf(h); q.kick(self.base);
        loop { if q.poll_used().is_some() { break; } core::hint::spin_loop(); }

        unsafe {
            let st = q.descriptors.add(s as usize);
            let st_v = hhdm + (*st).addr;
            let status = core::ptr::read_volatile(st_v as *const u8);
            if status != 0 { return Err("read failed"); }
        }

        q.free_desc(h); q.free_desc(d); q.free_desc(s);
        Ok(())
    }

    pub fn write_sectors(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        if !self.ready { return Err("not ready"); }
        let q = self.queue.as_mut().ok_or("no queue")?;
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);

        let h = q.alloc_desc().ok_or("no desc")?;
        let d = q.alloc_desc().ok_or("no desc")?;
        let s = q.alloc_desc().ok_or("no desc")?;

        unsafe {
            let hdr = q.descriptors.add(h as usize);
            let hdr_v = hhdm + (*hdr).addr;
            let req = hdr_v as *mut VirtioBlkReq;
            (*req).req_type = VIRTIO_BLK_T_OUT; (*req).sector = sector;
            (*hdr).len = 16; (*hdr).flags = VRING_DESC_F_NEXT; (*hdr).next = d;

            let dat = q.descriptors.add(d as usize);
            (*dat).addr = (buf.as_ptr() as u64) - hhdm;
            (*dat).len = buf.len() as u32;
            (*dat).flags = VRING_DESC_F_NEXT;
            (*dat).next = s;

            let st = q.descriptors.add(s as usize);
            let st_v = hhdm + (*st).addr;
            (*st).addr = (st_v - hhdm) as u64;
            (*st).len = 1; (*st).flags = VRING_DESC_F_WRITE; (*st).next = 0xFFFF;
        }

        q.add_buf(h); q.kick(self.base);
        loop { if q.poll_used().is_some() { break; } core::hint::spin_loop(); }

        unsafe {
            let st = q.descriptors.add(s as usize);
            let st_v = hhdm + (*st).addr;
            let status = core::ptr::read_volatile(st_v as *const u8);
            if status != 0 { return Err("write failed"); }
        }

        q.free_desc(h); q.free_desc(d); q.free_desc(s);
        Ok(())
    }

    pub fn is_ready(&self) -> bool { self.ready }
    pub fn disk_size(&self) -> u64 { self.disk_size }
}

static VIRTIO_BLK: Mutex<Option<VirtioBlock>> = Mutex::new(None);

pub fn probe_devices() {
    let bases: [u64; 4] = [0x0a000000, 0x0a001000, 0x0a002000, 0x0a003000];
    for &base in &bases {
        unsafe {
            let magic = core::ptr::read_volatile(base as *const u32);
            if magic == MAGIC {
                let mut dev = VirtioBlock::new(base);
                match dev.probe() {
                    Ok(()) => { *VIRTIO_BLK.lock() = Some(dev); return; }
                    Err(e) => { crate::println!("[VIRTIO-BLK] {:#x}: {}", base, e); }
                }
            }
        }
    }
    crate::println!("[VIRTIO-BLK] No block devices found");
}

pub fn read_sector(sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    let mut dev = VIRTIO_BLK.lock();
    dev.as_mut().ok_or("no device")?.read_sectors(sector, buf)
}

pub fn write_sector(sector: u64, buf: &[u8]) -> Result<(), &'static str> {
    let mut dev = VIRTIO_BLK.lock();
    dev.as_mut().ok_or("no device")?.write_sectors(sector, buf)
}

/// Реализация DiskIo для VirtIO блочного устройства
impl crate::fs::DiskIo for VirtioBlock {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        self.read_sectors(sector, buf).map_err(|_| crate::fs::vfs::FsError::IoError)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), crate::fs::vfs::FsError> {
        self.write_sectors(sector, buf).map_err(|_| crate::fs::vfs::FsError::IoError)
    }
}
