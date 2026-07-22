// VirtIO-GPU драйвер — управление GPU через VirtIO MMIO транспорт.
//
// Протокол: control queue (one queue), команды-ответы через VRing.
// Поддержка 2D framebuffer для вывода консоли на виртуальный GPU.

use crate::println;
use spin::Mutex;

// VirtIO-GPU: типы команд
const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x100;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x101;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x102;
const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x103;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x104;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x110;
const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x111;
const VIRTIO_GPU_CMD_GET_CAPSET_INFO: u32 = 0x120;

// Ответы
const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;

// Форматы пикселей
const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 1;

// MMIO регистры
const MMIO_MAGIC: u64 = 0x00;
const MMIO_VERSION: u64 = 0x04;
const MMIO_DEVICE_ID: u64 = 0x08;
const MMIO_DEVICE_FEATURES: u64 = 0x10;
const MMIO_DRIVER_FEATURES: u64 = 0x20;
const MMIO_QUEUE_SEL: u64 = 0x30;
const MMIO_QUEUE_NUM_MAX: u64 = 0x34;
const MMIO_QUEUE_READY: u64 = 0x44;
const MMIO_QUEUE_NOTIFY: u64 = 0x50;
const MMIO_STATUS: u64 = 0x70;
const MMIO_QUEUE_DESC_LOW: u64 = 0x80;
const MMIO_QUEUE_DESC_HIGH: u64 = 0x84;
const MMIO_QUEUE_DRIVER_LOW: u64 = 0x90;
const MMIO_QUEUE_DRIVER_HIGH: u64 = 0x94;
const MMIO_QUEUE_DEVICE_LOW: u64 = 0xA0;
const MMIO_QUEUE_DEVICE_HIGH: u64 = 0xA4;

const VIRTIO_MAGIC: u32 = 0x74726976;
const VIRTIO_VERSION: u32 = 2;
const VIRTIO_DEVICE_ID_GPU: u32 = 16;

const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;

const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

const GPU_MAX_RESPONSE: usize = 256;

// ===== Упакованные структуры =====

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuCtrlHdr {
    type_: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    padding: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuDisplayOne {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    enabled: u32,
    flags: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuRespDisplayInfo {
    hdr: VirtioGpuCtrlHdr,
    display: [VirtioGpuDisplayOne; 16],
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuResourceCreate2d {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuSetScanout {
    hdr: VirtioGpuCtrlHdr,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    scanout_id: u32,
    resource_id: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuTransfer {
    hdr: VirtioGpuCtrlHdr,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    resource_id: u32,
    padding: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuResourceAttachBacking {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtioGpuMemEntry {
    addr: u64,
    length: u32,
    padding: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

// ===== VirtQueue (адаптирована из virtio_block) =====

struct VirtQueue {
    descriptors: *mut VirtqDesc,
    avail_ring: *mut u16,
    used_ring: *mut u32,
    queue_size: u16,
    free_head: u16,
    num_free: u16,
    next_avail: u16,
    base_phys: u64,
}

unsafe impl Send for VirtQueue {}

impl VirtQueue {
    fn new(queue_size: u16, hhdm: u64) -> Option<Self> {
        let desc_sz = (queue_size as usize) * core::mem::size_of::<VirtqDesc>();
        let avail_sz = 6 + (queue_size as usize) * 2;
        let used_sz = 6 + (queue_size as usize) * 8;
        let total = (desc_sz + avail_sz + used_sz + 4095) & !4095;

        let phys = crate::memory::pmm::alloc_contiguous(total / 4096)?;
        let virt = hhdm + phys as u64;

        unsafe {
            let ptr = virt as *mut u8;
            for i in 0..total {
                core::ptr::write_volatile(ptr.add(i), 0);
            }

            let descriptors = virt as *mut VirtqDesc;
            let avail_ring = (virt as *mut u8).add(desc_sz) as *mut u16;
            let used_ring = (virt as *mut u8).add(desc_sz + avail_sz) as *mut u32;

            let q = VirtQueue {
                descriptors,
                avail_ring,
                used_ring,
                queue_size,
                free_head: 0,
                num_free: queue_size,
                next_avail: 0,
                base_phys: phys as u64,
            };
            for i in 0..(queue_size - 1) {
                (*descriptors.add(i as usize)).next = i + 1;
            }
            (*descriptors.add((queue_size - 1) as usize)).next = 0xFFFF;
            Some(q)
        }
    }

    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let idx = self.free_head;
        unsafe {
            self.free_head = (*self.descriptors.add(idx as usize)).next;
        }
        self.num_free -= 1;
        Some(idx)
    }

    fn free_desc(&mut self, idx: u16) {
        unsafe {
            (*self.descriptors.add(idx as usize)).next = self.free_head;
        }
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
        unsafe {
            core::ptr::write_volatile((base + MMIO_QUEUE_NOTIFY) as *mut u32, 0);
        }
    }

    fn poll_used(&mut self) -> Option<u16> {
        unsafe {
            let used_idx = core::ptr::read_volatile(self.used_ring.add(1)) as u16;
            if used_idx != self.next_avail {
                let ring = (self.next_avail as usize) % (self.queue_size as usize);
                let id = core::ptr::read_volatile(self.used_ring.add(2 + ring * 2));
                self.next_avail += 1;
                Some(id as u16)
            } else {
                None
            }
        }
    }
}

// ===== VirtioGpu =====

pub struct VirtioGpu {
    base: u64,
    queue: VirtQueue,
    queue_size: u16,
    fb_phys: u64,
    fb_virt: *mut u32,
    width: u32,
    height: u32,
    pitch: u32,
    resource_id: u32,
    ready: bool,
}

unsafe impl Send for VirtioGpu {}
unsafe impl Sync for VirtioGpu {}

impl VirtioGpu {
    unsafe fn r32(&self, o: u64) -> u32 {
        core::ptr::read_volatile((self.base + o) as *const u32)
    }
    unsafe fn w32(&self, o: u64, v: u32) {
        core::ptr::write_volatile((self.base + o) as *mut u32, v);
    }
    unsafe fn w8(&self, o: u64, v: u8) {
        core::ptr::write_volatile((self.base + o) as *mut u8, v);
    }

    fn alloc_resource_id(&mut self) -> u32 {
        self.resource_id += 1;
        self.resource_id
    }

    /// Отправить команду и получить ответ через control queue.
    fn send_command(&mut self, cmd: &[u8], resp: &mut [u8]) -> Result<(), &'static str> {
        let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
        let q = &mut self.queue;

        // Выделяем два дескриптора: cmd + resp
        let cmd_desc = q.alloc_desc().ok_or("gpu: нет дескрипторов (cmd)")?;
        let resp_desc = q.alloc_desc().ok_or("gpu: нет дескрипторов (resp)")?;

        unsafe {
            // Дескриптор команды (device-readable)
            let cmd_phys = (cmd.as_ptr() as u64) - hhdm;
            let d = &mut *q.descriptors.add(cmd_desc as usize);
            d.addr = cmd_phys;
            d.len = cmd.len() as u32;
            d.flags = VRING_DESC_F_NEXT;
            d.next = resp_desc;

            // Дескриптор ответа (device-writable)
            let resp_phys = (resp.as_mut_ptr() as u64) - hhdm;
            let d = &mut *q.descriptors.add(resp_desc as usize);
            d.addr = resp_phys;
            d.len = resp.len() as u32;
            d.flags = VRING_DESC_F_WRITE;
            d.next = 0xFFFF;
        }

        q.add_buf(cmd_desc);
        q.kick(self.base);

        // Ожидаем ответ (таймаут ~1M итераций)
        let mut timeout = 0u32;
        loop {
            if q.poll_used().is_some() {
                break;
            }
            timeout += 1;
            if timeout > 1_000_000 {
                q.free_desc(cmd_desc);
                q.free_desc(resp_desc);
                return Err("gpu: таймаут ответа");
            }
            core::hint::spin_loop();
        }

        q.free_desc(cmd_desc);
        q.free_desc(resp_desc);
        Ok(())
    }

    /// Инициализация VirtIO-GPU: ACK + DRIVER, настройка queue, GET_DISPLAY_INFO,
    /// создание ресурса, привязка буфера, настройка scanout.
    fn probe(&mut self) -> Result<(), &'static str> {
        unsafe {
            // Проверяем magic + version + device_id
            let magic = self.r32(MMIO_MAGIC);
            if magic != VIRTIO_MAGIC {
                return Err("gpu: неверный magic");
            }
            let version = self.r32(MMIO_VERSION);
            if version != VIRTIO_VERSION {
                return Err("gpu: неверная версия VirtIO");
            }
            let device_id = self.r32(MMIO_DEVICE_ID);
            if device_id != VIRTIO_DEVICE_ID_GPU {
                return Err("gpu: не GPU device");
            }

            // ACK + DRIVER
            let status = VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER;
            self.w8(MMIO_STATUS, status);

            // Читаем features, отключаем все продвинутые
            let features = self.r32(MMIO_DEVICE_FEATURES);
            let _ = features;
            self.w32(MMIO_DRIVER_FEATURES, 0);

            // Настройка queue 0
            self.w32(MMIO_QUEUE_SEL, 0);
            self.queue_size = self.r32(MMIO_QUEUE_NUM_MAX) as u16;
            if self.queue_size == 0 {
                return Err("gpu: queue size 0");
            }
            // Ограничиваем размер очереди
            if self.queue_size > 32 {
                self.queue_size = 32;
            }

            let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);

            // Заполняем VRing нулями (на всякий случай — выделенные страницы уже нулевые)
            // К сожалению, VirtQueue::new уже это делает.

            // Виртуальная и физическая адреса VRing
            let desc_phys = self.queue.base_phys;
            let desc_sz = (self.queue_size as usize) * core::mem::size_of::<VirtqDesc>();
            let avail_phys = desc_phys + desc_sz as u64;
            let avail_sz = 6 + (self.queue_size as usize) * 2;
            let used_phys = avail_phys + avail_sz as u64;

            self.w32(MMIO_QUEUE_DESC_LOW, desc_phys as u32);
            self.w32(MMIO_QUEUE_DESC_HIGH, (desc_phys >> 32) as u32);
            self.w32(MMIO_QUEUE_DRIVER_LOW, avail_phys as u32);
            self.w32(MMIO_QUEUE_DRIVER_HIGH, (avail_phys >> 32) as u32);
            self.w32(MMIO_QUEUE_DEVICE_LOW, used_phys as u32);
            self.w32(MMIO_QUEUE_DEVICE_HIGH, (used_phys >> 32) as u32);
            self.w32(MMIO_QUEUE_READY, 1);

            println!(
                "[GPU] VirtIO-GPU найден: {:#x}, qsz={}",
                self.base, self.queue_size
            );

            // GET_DISPLAY_INFO
            self.get_display_info()?;

            // Вычисляем размер framebuffer
            let fb_size = (self.width as usize) * (self.height as usize) * 4;
            let fb_pages = (fb_size + 4095) / 4096;

            // Выделяем framebuffer через PMM
            let fb_phys = crate::memory::pmm::alloc_contiguous(fb_pages)
                .ok_or("gpu: не удалось выделить framebuffer")?;
            self.fb_phys = fb_phys as u64;
            self.fb_virt = (hhdm + fb_phys as u64) as *mut u32;

            // Очищаем framebuffer
            let fb_slice = core::slice::from_raw_parts_mut(self.fb_virt, (self.width * self.height) as usize);
            fb_slice.fill(0);

            // Определяем pitch (ширина в байтах, выровненная по 4)
            self.pitch = self.width * 4;

            // RESOURCE_CREATE_2D
            let res_id = self.alloc_resource_id();
            self.resource_create_2d(res_id, self.width, self.height)?;

            // RESOURCE_ATTACH_BACKING — привязываем физические страницы framebuffer
            self.resource_attach_backing(res_id, self.fb_phys, fb_size as u32)?;

            // SET_SCANOUT — подключаем ресурс к display head 0
            self.set_scanout(0, 0, 0, self.width, self.height, res_id)?;

            println!(
                "[GPU] Инициализация завершена: {}x{}, resource_id={}, fb_phys={:#x}",
                self.width, self.height, res_id, self.fb_phys
            );

            self.ready = true;
        }
        Ok(())
    }

    /// Запрос информации о дисплеях
    fn get_display_info(&mut self) -> Result<(), &'static str> {
        unsafe {
            let mut cmd = VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_GET_DISPLAY_INFO,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            };
            let mut resp = VirtioGpuRespDisplayInfo {
                hdr: VirtioGpuCtrlHdr {
                    type_: 0,
                    flags: 0,
                    fence_id: 0,
                    ctx_id: 0,
                    padding: 0,
                },
                display: [VirtioGpuDisplayOne {
                    x: 0, y: 0, width: 0, height: 0, enabled: 0, flags: 0,
                }; 16],
            };

            self.send_command(
                core::slice::from_raw_parts(&cmd as *const _ as *const u8, core::mem::size_of::<VirtioGpuCtrlHdr>()),
                core::slice::from_raw_parts_mut(&mut resp as *mut _ as *mut u8, core::mem::size_of::<VirtioGpuRespDisplayInfo>()),
            )?;

            let resp_type = core::ptr::read_volatile(&resp as *const _ as *const u32);
            if resp_type != VIRTIO_GPU_RESP_OK_DISPLAY_INFO {
                return Err("gpu: GET_DISPLAY_INFO: неверный ответ");
            }

            // Ищем первый активный дисплей
            let mut found = false;
            for i in 0..16 {
                let d = resp.display[i];
                let d_enabled = d.enabled;
                let d_width = d.width;
                let d_height = d.height;
                let d_x = d.x;
                let d_y = d.y;
                if d_enabled != 0 && d_width > 0 && d_height > 0 {
                    self.width = d_width;
                    self.height = d_height;
                    println!(
                        "[GPU] Дисплей {}: {}x{} +({},+{})",
                        i, d_width, d_height, d_x, d_y
                    );
                    found = true;
                    break;
                }
            }

            if !found {
                // Fallback: 1024x768
                println!("[GPU] Нет активных дисплеев, используем 1024x768");
                self.width = 1024;
                self.height = 768;
            }

            Ok(())
        }
    }

    /// Создание 2D ресурса
    fn resource_create_2d(&mut self, resource_id: u32, width: u32, height: u32) -> Result<(), &'static str> {
        let cmd = VirtioGpuResourceCreate2d {
            hdr: VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_RESOURCE_CREATE_2D,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            },
            resource_id,
            format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
            width,
            height,
        };
        let mut resp_buf = [0u8; GPU_MAX_RESPONSE];
        self.send_command(
            unsafe {
                core::slice::from_raw_parts(
                    &cmd as *const _ as *const u8,
                    core::mem::size_of::<VirtioGpuResourceCreate2d>(),
                )
            },
            &mut resp_buf,
        )?;
        let resp_type = unsafe { core::ptr::read_volatile(resp_buf.as_ptr() as *const u32) };
        if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
            return Err("gpu: RESOURCE_CREATE_2D: неверный ответ");
        }
        Ok(())
    }

    /// Привязка физических страниц к ресурсу
    fn resource_attach_backing(&mut self, resource_id: u32, fb_phys: u64, fb_size: u32) -> Result<(), &'static str> {
        let cmd = VirtioGpuResourceAttachBacking {
            hdr: VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            },
            resource_id,
            nr_entries: 1,
        };
        let entry = VirtioGpuMemEntry {
            addr: fb_phys,
            length: fb_size,
            padding: 0,
        };

        let cmd_size = core::mem::size_of::<VirtioGpuResourceAttachBacking>();
        let entry_size = core::mem::size_of::<VirtioGpuMemEntry>();
        let total_size = cmd_size + entry_size;

        let mut cmd_buf = [0u8; 128];
        unsafe {
            core::ptr::copy_nonoverlapping(
                &cmd as *const _ as *const u8,
                cmd_buf.as_mut_ptr(),
                cmd_size,
            );
            core::ptr::copy_nonoverlapping(
                &entry as *const _ as *const u8,
                cmd_buf.as_mut_ptr().add(cmd_size),
                entry_size,
            );
        }

        let mut resp_buf = [0u8; GPU_MAX_RESPONSE];
        self.send_command(&cmd_buf[..total_size], &mut resp_buf)?;
        let resp_type = unsafe { core::ptr::read_volatile(resp_buf.as_ptr() as *const u32) };
        if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
            return Err("gpu: RESOURCE_ATTACH_BACKING: неверный ответ");
        }
        Ok(())
    }

    /// Подключение ресурса к scanout
    fn set_scanout(&mut self, scanout_id: u32, x: u32, y: u32, width: u32, height: u32, resource_id: u32) -> Result<(), &'static str> {
        let cmd = VirtioGpuSetScanout {
            hdr: VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_SET_SCANOUT,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            },
            x,
            y,
            width,
            height,
            scanout_id,
            resource_id,
        };
        let mut resp_buf = [0u8; GPU_MAX_RESPONSE];
        self.send_command(
            unsafe {
                core::slice::from_raw_parts(
                    &cmd as *const _ as *const u8,
                    core::mem::size_of::<VirtioGpuSetScanout>(),
                )
            },
            &mut resp_buf,
        )?;
        let resp_type = unsafe { core::ptr::read_volatile(resp_buf.as_ptr() as *const u32) };
        if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
            return Err("gpu: SET_SCANOUT: неверный ответ");
        }
        Ok(())
    }

    /// Передать изменённую область framebuffer на GPU (TRANSFER_TO_HOST_2D + FLUSH)
    pub fn flush_region(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if !self.ready {
            return;
        }
        let res_id = self.resource_id;

        // TRANSFER_TO_HOST_2D
        let xfer = VirtioGpuTransfer {
            hdr: VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            },
            x, y, width: w, height: h,
            resource_id: res_id,
            padding: 0,
        };
        let mut resp_buf = [0u8; GPU_MAX_RESPONSE];
        unsafe {
            let _ = self.send_command(
                core::slice::from_raw_parts(
                    &xfer as *const _ as *const u8,
                    core::mem::size_of::<VirtioGpuTransfer>(),
                ),
                &mut resp_buf,
            );
        }

        // RESOURCE_FLUSH
        let flush = VirtioGpuTransfer {
            hdr: VirtioGpuCtrlHdr {
                type_: VIRTIO_GPU_CMD_RESOURCE_FLUSH,
                flags: 0,
                fence_id: 0,
                ctx_id: 0,
                padding: 0,
            },
            x, y, width: w, height: h,
            resource_id: res_id,
            padding: 0,
        };
        let mut resp_buf2 = [0u8; GPU_MAX_RESPONSE];
        unsafe {
            let _ = self.send_command(
                core::slice::from_raw_parts(
                    &flush as *const _ as *const u8,
                    core::mem::size_of::<VirtioGpuTransfer>(),
                ),
                &mut resp_buf2,
            );
        }
    }

    /// Полный flush всего framebuffer
    pub fn flush_all(&mut self) {
        let w = self.width;
        let h = self.height;
        self.flush_region(0, 0, w, h);
    }

    pub fn fb_virt(&self) -> *mut u32 {
        self.fb_virt
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pitch(&self) -> u32 {
        self.pitch
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }
}

// ===== Статическая переменная GPU =====

static GPU: Mutex<Option<VirtioGpu>> = Mutex::new(None);

pub fn get_gpu() -> spin::MutexGuard<'static, Option<VirtioGpu>> {
    GPU.lock()
}

/// Инициализация VirtIO-GPU драйвера (вызывается из drivers::init)
pub fn init() {
    println!("[GPU] Поиск VirtIO-GPU...");

    // Ищем VirtIO-GPU через PCI: vendor=0x1AF4, device=0x1050
    // или по class 0x03 (Display) subclass 0x00
    let pci_dev = crate::drivers::pci::find_device_by_id(0x1AF4, 0x1050)
        .or_else(|| crate::drivers::pci::find_device(0x03, 0x00));

    let pci_dev = match pci_dev {
        Some(d) => d,
        None => {
            println!("[GPU] VirtIO-GPU не найден в PCI");
            return;
        }
    };

    println!(
        "[GPU] PCI: {:02X}:{:02X}.{} {:04X}:{:04X}",
        pci_dev.bus, pci_dev.dev, pci_dev.func,
        pci_dev.vendor_id, pci_dev.device_id
    );

    // Включаем Bus Master
    unsafe {
        crate::drivers::pci::enable_bus_master(pci_dev.bus, pci_dev.dev, pci_dev.func);
    }

    // VirtIO-GPU на q35: VirtIO config обычно в BAR2 (BAR0 = VGA framebuffer)
    // Проверяем BAR0 и BAR2 — не сканируем все 6 (может вызвать fault)
    let hhdm = crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed);
    let bar0 = crate::drivers::pci::bar_phys(pci_dev.bus, pci_dev.dev, pci_dev.func, 0);
    let bar2 = crate::drivers::pci::bar_phys(pci_dev.bus, pci_dev.dev, pci_dev.func, 2);

    // Определяем верхний физический адрес из memory map
    let max_phys = match crate::MEMORY_MAP_REQUEST.response() {
        Some(mm) => mm.entries().iter()
            .map(|e| e.base + e.length)
            .max()
            .unwrap_or(0x1_0000_0000),
        None => 0x1_0000_0000,
    };

    let mut mmio_base: u64 = 0;

    // Проверяем BAR2首先 (обычно VirtIO config)
    if bar2 != 0 && bar2 < max_phys {
        let magic = unsafe { core::ptr::read_volatile((hhdm + bar2) as *const u32) };
        if magic == VIRTIO_MAGIC {
            mmio_base = bar2;
        }
    }

    // Если BAR2 не подошёл — проверяем BAR0
    if mmio_base == 0 && bar0 != 0 && bar0 < max_phys {
        let magic = unsafe { core::ptr::read_volatile((hhdm + bar0) as *const u32) };
        if magic == VIRTIO_MAGIC {
            mmio_base = bar0;
        }
    }

    if mmio_base == 0 {
        println!("[GPU] VirtIO MMIO не найден (BAR0={:#x} BAR2={:#x})", bar0, bar2);
        return;
    }

    let mut gpu = VirtioGpu {
        base: hhdm + mmio_base,
        queue: match VirtQueue::new(32, hhdm) {
            Some(q) => q,
            None => {
                println!("[GPU] Не удалось создать VirtQueue");
                return;
            }
        },
        queue_size: 0,
        fb_phys: 0,
        fb_virt: core::ptr::null_mut(),
        width: 0,
        height: 0,
        pitch: 0,
        resource_id: 0,
        ready: false,
    };

    match gpu.probe() {
        Ok(()) => {
            println!("[GPU] VirtIO-GPU успешно инициализирован");
            *GPU.lock() = Some(gpu);
        }
        Err(e) => {
            println!("[GPU] Ошибка инициализации: {}", e);
        }
    }
}

/// Переключить консоль на GPU framebuffer, если GPU доступен
pub fn try_switch_console(console: &mut crate::framebuffer::Console) {
    let mut gpu_guard = GPU.lock();
    if let Some(gpu) = gpu_guard.as_mut() {
        if gpu.is_ready() {
            let fb = gpu.fb_virt();
            let w = gpu.width() as usize;
            let h = gpu.height() as usize;
            let pitch_words = gpu.pitch() as usize / 4;
            console.switch_framebuffer(fb, w, h, pitch_words);
            println!(
                "[GPU] Консоль переключена на VirtIO-GPU framebuffer {}x{}",
                w, h
            );
        }
    }
}

/// Transfer + flush полного framebuffer (вызывается после flip консоли)
pub fn flush_full_framebuffer() {
    let mut gpu_guard = GPU.lock();
    if let Some(gpu) = gpu_guard.as_mut() {
        gpu.flush_all();
    }
}
