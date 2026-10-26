// USB Mass Storage — Bulk-Only Transport (BOT) + минимальный SCSI.
//
// Поддержаны команды, которых достаточно для FAT32:
//   INQUIRY, TEST UNIT READY, READ CAPACITY(10),
//   READ(10), WRITE(10), SYNCHRONIZE CACHE(10).
//
// Один SCSI-приказ = CBW (31 байт, bulk OUT) → data stage → CSW (13 байт,
// bulk IN). После STALL/timeout выполняется Bulk-Only Reset + ClearFeature(HALT)
// и команда повторяется один раз: иначе BOT-сессия остаётся рассогласованной.
//
// Драйвер не знает о VFS: `UsbDisk` реализует `DiskIo`, а регистрацией в
// `DiskManager` занимается `fs::blockdev`.

use super::xhci::{self, StorageInfo, UsbError};
use crate::fs::disk::DiskIo;
use crate::fs::vfs::FsError;
use crate::println;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC" little-endian
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS" little-endian
const SECTOR_SIZE: usize = 512;

/// Максимум секторов в одной SCSI-команде (32 КиБ данных).
const MAX_SECTORS_PER_CMD: usize = 64;

/// Паспорт найденного накопителя.
#[derive(Clone)]
pub struct DiskInfo {
    pub controller: usize,
    pub slot: u8,
    pub vendor: String,
    pub product: String,
    pub revision: String,
    pub sectors: u64,
    /// Секторов за одну SCSI-команду (ограничено `wMaxPacketSize` контроллера).
    pub sectors_per_cmd: usize,
}

impl DiskInfo {
    pub fn size_mb(&self) -> u64 {
        (self.sectors * SECTOR_SIZE as u64) / (1024 * 1024)
    }

    pub fn model(&self) -> String {
        let vendor = self.vendor.trim();
        let product = self.product.trim();
        if vendor.is_empty() {
            String::from(product)
        } else {
            alloc::format!("{} {}", vendor, product)
        }
    }
}

/// USB-накопитель, доступный через `DiskIo`.
pub struct UsbDisk {
    controller: usize,
    slot: u8,
    sectors_per_cmd: usize,
    tag: u32,
}

impl UsbDisk {
    /// Открывает накопитель и проверяет его SCSI-командами.
    pub fn new(info: &StorageInfo) -> Result<(Self, DiskInfo), FsError> {
        let mut disk = UsbDisk {
            controller: info.controller,
            slot: info.slot,
            sectors_per_cmd: (info.max_transfer / SECTOR_SIZE).clamp(1, MAX_SECTORS_PER_CMD),
            tag: 0,
        };

        if disk.test_unit_ready().is_err() {
            // Первый TEST UNIT READY после включения часто возвращает
            // «unit attention»; второй запрос обычно проходит.
            disk.test_unit_ready()?;
        }

        let mut inquiry = [0u8; 36];
        disk.execute(&[0x12, 0, 0, 0, 36, 0], Some(&mut inquiry), true)?;
        let vendor = ascii_field(&inquiry[8..16]);
        let product = ascii_field(&inquiry[16..32]);
        let revision = ascii_field(&inquiry[32..36]);

        let mut capacity = [0u8; 8];
        disk.execute(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], Some(&mut capacity), true)?;
        let last_lba = u32::from_be_bytes([capacity[0], capacity[1], capacity[2], capacity[3]]);
        let block_size =
            u32::from_be_bytes([capacity[4], capacity[5], capacity[6], capacity[7]]);
        if block_size != SECTOR_SIZE as u32 {
            println!(
                "[USB-STORAGE] unsupported block size {} (expected {})",
                block_size, SECTOR_SIZE
            );
            return Err(FsError::NotSupported);
        }

        let sectors = last_lba as u64 + 1;
        let info = DiskInfo {
            controller: info.controller,
            slot: info.slot,
            vendor,
            product,
            revision,
            sectors,
            sectors_per_cmd: disk.sectors_per_cmd,
        };
        Ok((disk, info))
    }

    fn test_unit_ready(&mut self) -> Result<(), FsError> {
        self.execute(&[0x00, 0, 0, 0, 0, 0], None, true).map(|_| ())
    }

    /// Выполняет SCSI-команду с однократным recovery при ошибке шины.
    fn execute(
        &mut self,
        cdb: &[u8],
        buf: Option<&mut [u8]>,
        dir_in: bool,
    ) -> Result<(), FsError> {
        let mut data = buf;
        match self.bot(cdb, data.as_deref_mut(), dir_in) {
            Ok(()) => Ok(()),
            Err(first) => {
                // Recovery: Bulk-Only Reset + очистка halt'ов, затем повтор.
                let _ = xhci::resolve_stall(self.controller, self.slot);
                match self.bot(cdb, data, dir_in) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(first),
                }
            }
        }
    }

    /// Один цикл Bulk-Only Transport.
    fn bot(&mut self, cdb: &[u8], data: Option<&mut [u8]>, dir_in: bool) -> Result<(), FsError> {
        let mut cbw = [0u8; 31];
        self.tag = self.tag.wrapping_add(1);

        let transfer_len = data.as_ref().map(|b| b.len()).unwrap_or(0) as u32;
        cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        cbw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&transfer_len.to_le_bytes());
        cbw[12] = if dir_in { 0x80 } else { 0x00 };
        cbw[13] = 0; // LUN 0
        cbw[14] = cdb.len() as u8;
        cbw[15..15 + cdb.len()].copy_from_slice(cdb);

        // 1. Command stage
        let sent = xhci::bulk(self.controller, self.slot, false, &mut cbw)?;
        if sent != cbw.len() {
            return Err(FsError::IoError);
        }

        // 2. Data stage
        if let Some(buffer) = data {
            if !buffer.is_empty() {
                let done = xhci::bulk(self.controller, self.slot, dir_in, buffer)?;
                // Размер известен заранее, поэтому короткий transfer — ошибка.
                if done != buffer.len() {
                    return Err(FsError::IoError);
                }
            }
        }

        // 3. Status stage
        let mut csw = [0u8; 13];
        let done = xhci::bulk(self.controller, self.slot, true, &mut csw)?;
        if done != csw.len() {
            return Err(FsError::IoError);
        }
        if u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]) != CSW_SIGNATURE {
            return Err(FsError::IoError);
        }
        // Тег должен совпадать: иначе пришёл CSW от предыдущей команды.
        if u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]) != self.tag {
            return Err(FsError::IoError);
        }
        // 0 = passed, 1 = failed, 2 = phase error.
        if csw[12] != 0 {
            return Err(FsError::IoError);
        }
        Ok(())
    }
}

impl From<UsbError> for FsError {
    fn from(_: UsbError) -> Self {
        FsError::IoError
    }
}

impl DiskIo for UsbDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        if buf.is_empty() {
            return Ok(());
        }
        if buf.len() % SECTOR_SIZE != 0 {
            return Err(FsError::InvalidPath);
        }

        let mut done = 0usize;
        while done < buf.len() {
            let sectors = ((buf.len() - done) / SECTOR_SIZE).min(self.sectors_per_cmd);
            let lba = sector + (done / SECTOR_SIZE) as u64;
            let cdb = read10_cdb(0x28, lba, sectors as u16);
            let chunk = &mut buf[done..done + sectors * SECTOR_SIZE];
            self.execute(&cdb, Some(chunk), true)?;
            done += sectors * SECTOR_SIZE;
        }
        Ok(())
    }

    fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        if buf.is_empty() {
            return Ok(());
        }
        if buf.len() % SECTOR_SIZE != 0 {
            return Err(FsError::InvalidPath);
        }

        let mut done = 0usize;
        while done < buf.len() {
            let sectors = ((buf.len() - done) / SECTOR_SIZE).min(self.sectors_per_cmd);
            let lba = sector + (done / SECTOR_SIZE) as u64;
            let cdb = read10_cdb(0x2A, lba, sectors as u16);
            let mut chunk = buf[done..done + sectors * SECTOR_SIZE].to_vec();
            self.execute(&cdb, Some(&mut chunk), false)?;
            done += sectors * SECTOR_SIZE;
        }

        // SYNCHRONIZE CACHE(10): без него данные остаются в кэше устройства
        // и теряются при выдёргивании флешки.
        let cdb = read10_cdb(0x35, sector, (buf.len() / SECTOR_SIZE) as u16);
        let _ = self.execute(&cdb, None, true);
        Ok(())
    }
}

/// CDB для READ(10)/WRITE(10)/SYNCHRONIZE CACHE(10) — раскладка одинаковая.
fn read10_cdb(opcode: u8, lba: u64, count: u16) -> [u8; 10] {
    let mut cdb = [0u8; 10];
    cdb[0] = opcode;
    cdb[2..6].copy_from_slice(&(lba as u32).to_be_bytes());
    cdb[7..9].copy_from_slice(&count.to_be_bytes());
    cdb
}

fn ascii_field(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b >= 0x20 && b < 0x7F {
            out.push(b as char);
        }
    }
    out
}

// ==========================================
// Реестр накопителей
// ==========================================

static DISKS: Mutex<Vec<DiskInfo>> = Mutex::new(Vec::new());

/// Опросить все mass storage устройства и зарегистрировать накопители.
/// Возвращает число найденных дисков.
pub fn probe() -> usize {
    let devices = xhci::storage_devices();
    if devices.is_empty() {
        return 0;
    }

    let mut disks = DISKS.lock();
    for info in devices {
        if disks
            .iter()
            .any(|d| d.controller == info.controller && d.slot == info.slot)
        {
            continue;
        }
        match UsbDisk::new(&info) {
            Ok((_, disk_info)) => {
                println!(
                    "[USB-STORAGE] ctrl{} slot{}: {} {} ({} sectors, {} MiB)",
                    info.controller,
                    info.slot,
                    disk_info.model(),
                    disk_info.revision,
                    disk_info.sectors,
                    disk_info.size_mb()
                );
                disks.push(disk_info);
            }
            Err(error) => println!(
                "[USB-STORAGE] ctrl{} slot{}: probe failed ({:?})",
                info.controller, info.slot, error
            ),
        }
    }
    disks.len()
}

/// Список накопителей (для `diag usb`).
pub fn disks() -> Vec<DiskInfo> {
    DISKS.lock().clone()
}

/// Открыть накопитель как `DiskIo` (используется `fs::blockdev`).
pub fn open(index: usize) -> Option<Box<dyn DiskIo>> {
    let info = DISKS.lock().get(index)?.clone();
    Some(Box::new(UsbDisk {
        controller: info.controller,
        slot: info.slot,
        sectors_per_cmd: info.sectors_per_cmd,
        tag: 0,
    }))
}
