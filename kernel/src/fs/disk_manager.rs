// fs/disk_manager.rs — реестр блочных устройств и их разделов.
//
// Один физический диск (например, SATA-порт AHCI) может содержать несколько
// разделов. Файловая система монтируется не с диска, а с раздела, поэтому
// менеджер хранит «окна» (offset + размер) поверх общего устройства.
//
// Устройство хранится как `Arc<Mutex<Box<dyn DiskIo>>>`: так можно выдать
// файловой системе независимое окно, не отбирая диск у менеджера.

use super::disk::DiskIo;
use super::partition::PartitionDisk;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::format;
use spin::Mutex;

/// Запись реестра: диск или раздел.
pub struct DiskEntry {
    /// Имя вида `sda`, `sda1`, `ram0`.
    pub name: String,
    /// `"disk"` или `"partition"`.
    pub kind: &'static str,
    /// Размер в 512-байтных секторах.
    pub sectors: u64,
    /// Начало окна в секторах родительского устройства.
    pub offset: u64,
    /// Определённый тип ФС: `fat32`, `ntfs`, `ext2`, `unknown`.
    pub fs_type: &'static str,
    /// Общее устройство ввода-вывода.
    pub disk: Arc<Mutex<Box<dyn DiskIo>>>,
}

impl DiskEntry {
    /// Смещение в байтах (для вывода в `df`/`lsblk`).
    pub fn offset_bytes(&self) -> u64 { self.offset * 512 }

    /// Размер в мебибайтах.
    pub fn size_mb(&self) -> u64 { (self.sectors * 512) / (1024 * 1024) }
}

/// Менеджер дисков для монтирования
pub struct DiskManager {
    disks: Vec<DiskEntry>,
    next_ram: u32,
}

impl DiskManager {
    pub fn new() -> Self {
        Self { disks: Vec::new(), next_ram: 0 }
    }

    /// Добавить «безымянный» диск (используется командой `test-disk ram`).
    ///
    /// Размер неизвестен, поэтому ставим 0 — такой диск нельзя разбить на
    /// разделы, но можно смонтировать целиком.
    pub fn add_disk(&mut self, disk: Box<dyn DiskIo>) -> usize {
        let name = format!("ram{}", self.next_ram);
        self.next_ram += 1;
        self.disks.push(DiskEntry {
            name,
            kind: "disk",
            sectors: 0,
            offset: 0,
            fs_type: "unknown",
            disk: Arc::new(Mutex::new(disk)),
        });
        self.disks.len() - 1
    }

    /// Зарегистрировать устройство с известными параметрами.
    pub fn add_entry(&mut self, entry: DiskEntry) -> usize {
        self.disks.push(entry);
        self.disks.len() - 1
    }

    /// Открыть окно на устройство/раздел по индексу.
    ///
    /// Возвращает `Box<dyn DiskIo>`, который можно передать в `Fat32Fs::new` и
    /// т.п. Для «безымянных» дисков (sectors == 0) границы не проверяются.
    pub fn open(&self, index: usize) -> Option<Box<dyn DiskIo>> {
        let e = self.disks.get(index)?;
        if e.sectors == 0 {
            return Some(Box::new(WholeDisk::new(e.disk.clone())));
        }
        Some(Box::new(PartitionDisk::new(e.disk.clone(), e.offset, e.sectors)))
    }

    /// Информация для `mount`/`df`: (имя, вид, сектора, тип ФС).
    pub fn info(&self, index: usize) -> Option<(&str, &'static str, u64, &'static str)> {
        let e = self.disks.get(index)?;
        Some((e.name.as_str(), e.kind, e.sectors, e.fs_type))
    }

    /// Все записи (для листинга).
    pub fn entries(&self) -> &[DiskEntry] { &self.disks }

    /// Найти индекс по имени (`sda1`).
    pub fn find_by_name(&self, name: &str) -> Option<usize> {
        self.disks.iter().position(|e| e.name == name)
    }

    /// Количество дисков
    pub fn count(&self) -> usize {
        self.disks.len()
    }
}

impl Default for DiskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Прямой доступ к устройству без ограничения границ (для `sectors == 0`).
struct WholeDisk {
    inner: Arc<Mutex<Box<dyn DiskIo>>>,
}

impl WholeDisk {
    fn new(inner: Arc<Mutex<Box<dyn DiskIo>>>) -> Self { Self { inner } }
}

impl DiskIo for WholeDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().read(sector, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().write(sector, buf)
    }
}

