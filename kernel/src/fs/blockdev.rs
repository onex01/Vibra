// fs/blockdev.rs — обнаружение блочных устройств и их разделов.
//
// Связывает драйвер AHCI с VFS: создаёт по одному `AhciDisk` на активный
// SATA-порт, читает таблицу разделов (MBR/GPT) и регистрирует в DiskManager
// как `sda`, `sda1`, `sda2`, ...

use super::disk::DiskIo;
use super::disk_manager::{DiskEntry, DiskManager};
use super::partition::{detect_fs_type, part_name, read_partitions};
use alloc::boxed::Box;
use alloc::sync::Arc;
use spin::Mutex;

/// Обнаружить SATA-диски и их разделы.
///
/// Возвращает количество добавленных записей (диски + разделы).
pub fn probe(mgr: &mut DiskManager) -> usize {
    let ports = crate::drivers::ahci::active_ports();
    if ports.is_empty() { return 0; }

    let mut added = 0usize;

    for (idx, port) in ports.iter().enumerate() {
        // Имя в стиле Linux: sda, sdb, ...
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
            disk_name, sectors,
            (sectors * 512) / (1024 * 1024),
            if model.is_empty() { alloc::string::String::new() } else { alloc::format!(" [{}]", model) }
        );

        let shared: Arc<Mutex<Box<dyn DiskIo>>> =
            Arc::new(Mutex::new(Box::new(crate::drivers::ahci::AhciDisk::new(*port))));

        // Тип ФС самого диска (актуально для «superfloppy» без таблицы).
        let disk_fs = {
            let mut view = SharedDisk::new(shared.clone());
            detect_fs_type(&mut view)
        };

        mgr.add_entry(DiskEntry {
            name: disk_name.clone(),
            kind: "disk",
            sectors,
            offset: 0,
            fs_type: disk_fs,
            disk: shared.clone(),
        });
        added += 1;

        // Таблица разделов.
        let parts = {
            let mut view = SharedDisk::new(shared.clone());
            read_partitions(&mut view)
        };

        for p in parts {
            let name = part_name(&disk_name, p.index);
            let fs_type = {
                // detect_fs_type читает сектор 0 окна, поэтому нужен сдвиг —
                // делаем отдельное окно через PartitionDisk.
                let mut win = super::partition::PartitionDisk::new(
                    shared.clone(), p.start_lba, p.sectors,
                );
                detect_fs_type(&mut win)
            };

            crate::println!(
                "[BLK] {}: {} МиБ, тип {}, ФС {}",
                name, (p.sectors * 512) / (1024 * 1024), p.kind.as_str(), fs_type
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
    }

    added
}

/// Тонкая обёртка `Arc<Mutex<Box<dyn DiskIo>>>` в `DiskIo` для проб.
struct SharedDisk {
    inner: Arc<Mutex<Box<dyn DiskIo>>>,
}

impl SharedDisk {
    fn new(inner: Arc<Mutex<Box<dyn DiskIo>>>) -> Self { Self { inner } }
}

impl DiskIo for SharedDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().read(sector, buf)
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), super::vfs::FsError> {
        self.inner.lock().write(sector, buf)
    }
}
