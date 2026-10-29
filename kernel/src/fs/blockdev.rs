// fs/blockdev.rs — обнаружение блочных устройств и их разделов.
//
// Связывает драйверы (AHCI + USB mass storage) с VFS: создаёт по одному
// диску на активный SATA-порт и на каждый найденный USB-накопитель, читает
// таблицу разделов (MBR/GPT) и регистрирует в DiskManager как `sda`, `sda1`,
// `sdb`, ...
//
// Порядок имён: сначала SATA (`sda`...), затем USB продолжает буквы. На
// ThinkPad T450 внутренний SSD — sda, загрузочная флешка — sdb.

use super::disk::DiskIo;
use super::disk_manager::{DiskEntry, DiskManager};
use super::partition::{detect_fs_type, part_name, read_partitions};
use alloc::boxed::Box;
use alloc::sync::Arc;
use spin::Mutex;

/// Обнаружить блочные устройства и их разделы.
///
/// Возвращает количество добавленных записей (диски + разделы).
pub fn probe(mgr: &mut DiskManager) -> usize {
    let (added, disks) = probe_ahci(mgr);
    let (nvme_added, nvme_disks) = probe_nvme(mgr, disks);
    added + nvme_added + probe_usb(mgr, disks + nvme_disks)
}

/// SATA-диски: `(число записей, число дисков)`.
fn probe_ahci(mgr: &mut DiskManager) -> (usize, usize) {
    let ports = crate::drivers::ahci::active_ports();
    let mut added = 0usize;
    let mut disks = 0usize;

    for (idx, port) in ports.iter().enumerate() {
        let disk_name = alloc::format!("sd{}", (b'a' + (idx % 26) as u8) as char);

        let info = crate::drivers::ahci::identify_device(*port);
        let sectors = info.as_ref().map(|i| i.total_sectors).unwrap_or(0);
        let model = info.as_ref().map(|i| i.model.clone()).unwrap_or_default();

        if sectors == 0 {
            crate::println!("[BLK] {}: нет ответа на IDENTIFY, пропускаем", disk_name);
            continue;
        }

        crate::println!(
            "[BLK] {}: {} секторов ({} МиБ){}",
            disk_name,
            sectors,
            (sectors * 512) / (1024 * 1024),
            if model.is_empty() {
                alloc::string::String::new()
            } else {
                alloc::format!(" [{}]", model)
            }
        );

        let shared: Arc<Mutex<Box<dyn DiskIo>>> =
            Arc::new(Mutex::new(Box::new(crate::drivers::ahci::AhciDisk::with_sector_size(
                *port,
                info.as_ref().map(|i| i.sector_size).unwrap_or(512),
            ))));

        added += register(mgr, &disk_name, shared, sectors);
        disks += 1;
    }

    (added, disks)
}

/// NVMe-накопитель: один namespace, 512-байтные сектора для FS.
fn probe_nvme(mgr: &mut DiskManager, first_index: usize) -> (usize, usize) {
    let Some((blocks, block_size)) = crate::drivers::nvme::disk_info() else {
        return (0, 0);
    };
    let Some(disk) = crate::drivers::nvme::create_disk() else {
        return (0, 0);
    };
    if blocks == 0 || block_size == 0 {
        return (0, 0);
    }

    // FS/partition layer считает сектора 512-байтными; адаптер в NvmeDisk
    // переводит их в логические блоки устройства (в т.ч. 4Kn).
    let sectors = blocks.saturating_mul(block_size as u64) / 512;
    if sectors == 0 {
        return (0, 0);
    }

    let letter = (b'a' + (first_index % 26) as u8) as char;
    let disk_name = alloc::format!("sd{}", letter);
    crate::println!(
        "[BLK] {}: {} секторов ({} МиБ) [NVMe, {} B/block]",
        disk_name,
        sectors,
        (sectors * 512) / (1024 * 1024),
        block_size
    );

    let shared: Arc<Mutex<Box<dyn DiskIo>>> = Arc::new(Mutex::new(disk));
    (register(mgr, &disk_name, shared, sectors), 1)
}

/// USB-накопители (mass storage). Нумерация продолжает SATA.
fn probe_usb(mgr: &mut DiskManager, first_index: usize) -> usize {
    let infos = crate::drivers::usb::storage::disks();
    let mut added = 0usize;

    for (offset, info) in infos.iter().enumerate() {
        let letter = (b'a' + ((first_index + offset) % 26) as u8) as char;
        let disk_name = alloc::format!("sd{}", letter);

        let Some(disk) = crate::drivers::usb::storage::open(offset) else {
            continue;
        };
        if info.sectors == 0 {
            continue;
        }

        crate::println!(
            "[BLK] {}: {} секторов ({} МиБ) [USB {}]",
            disk_name,
            info.sectors,
            info.size_mb(),
            info.model()
        );

        let shared: Arc<Mutex<Box<dyn DiskIo>>> = Arc::new(Mutex::new(disk));
        added += register(mgr, &disk_name, shared, info.sectors);
    }

    added
}

/// Регистрирует диск и его разделы. Возвращает число добавленных записей.
fn register(
    mgr: &mut DiskManager,
    disk_name: &str,
    shared: Arc<Mutex<Box<dyn DiskIo>>>,
    sectors: u64,
) -> usize {
    let mut added = 0usize;

    // Тип ФС самого диска (актуально для «superfloppy» без таблицы).
    let disk_fs = {
        let mut view = SharedDisk::new(shared.clone());
        detect_fs_type(&mut view)
    };

    mgr.add_entry(DiskEntry {
        name: alloc::string::String::from(disk_name),
        kind: "disk",
        sectors,
        offset: 0,
        fs_type: disk_fs,
        disk: shared.clone(),
    });
    added += 1;

    let parts = {
        let mut view = SharedDisk::new(shared.clone());
        read_partitions(&mut view)
    };

    for p in parts {
        let name = part_name(disk_name, p.index);
        let fs_type = {
            // detect_fs_type читает сектор 0 окна, поэтому нужен сдвиг —
            // делаем отдельное окно через PartitionDisk.
            let mut win =
                super::partition::PartitionDisk::new(shared.clone(), p.start_lba, p.sectors);
            detect_fs_type(&mut win)
        };

        crate::println!(
            "[BLK] {}: {} МиБ, тип {}, ФС {}",
            name,
            (p.sectors * 512) / (1024 * 1024),
            p.kind.as_str(),
            fs_type
        );

        mgr.add_entry(DiskEntry {
            name,
            kind: "partition",
            sectors: p.sectors,
            offset: p.start_lba,
            fs_type,
            disk: shared.clone(),
        });
        added += 1;
    }

    added
}

/// Тонкая обёртка `Arc<Mutex<Box<dyn DiskIo>>>` в `DiskIo` для проб.
struct SharedDisk {
    inner: Arc<Mutex<Box<dyn DiskIo>>>,
}

impl SharedDisk {
    fn new(inner: Arc<Mutex<Box<dyn DiskIo>>>) -> Self {
        Self { inner }
    }
}

impl DiskIo for SharedDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().read(sector, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().write(sector, buf)
    }

    fn sector_size(&self) -> usize {
        self.inner.lock().sector_size()
    }

    fn read_lba(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().read_lba(lba, buf)
    }
}
