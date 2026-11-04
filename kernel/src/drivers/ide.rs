// IDE / PATA (PIIX-style) — PIO-чтение дисков в legacy-режиме.
//
// Зачем: на старых платах (например, AMD A8-5600K с SB-чипсетом) BIOS часто
// оставляет SATA-контроллер в режиме IDE. В этом режиме устройство выглядит
// как PCI class 0x01 / subclass 0x01, а не 0x01/0x06 (AHCI), и драйвер AHCI
// его просто не находит — диски «пропадают».
//
// Драйвер намеренно минимальный: IDENTIFY DEVICE, READ SECTORS (LBA28) и
// READ SECTORS EXT (LBA48), только PIO, только чтение. Этого достаточно,
// чтобы block layer увидел разделы и смонтировал ФС; запись не поддерживается.
//
// Прерывания не используются: ждём BSY/DRQ опросом статуса, поэтому драйвер
// работает и до `interrupts::enable()`, и на машинах, где IRQ14 не приходит.

use crate::println;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Базовые порты командного блока для двух каналов.
const PRIMARY_CMD: u16 = 0x1F0;
const PRIMARY_CTRL: u16 = 0x3F6;
const SECONDARY_CMD: u16 = 0x170;
const SECONDARY_CTRL: u16 = 0x376;

/// Регистры относительно командного блока.
const REG_DATA: u16 = 0;
const REG_ERROR: u16 = 1;
const REG_COUNT: u16 = 2;
const REG_LBA0: u16 = 3;
const REG_LBA1: u16 = 4;
const REG_LBA2: u16 = 5;
const REG_DRIVE: u16 = 6;
const REG_STATUS: u16 = 7;
const REG_COMMAND: u16 = 7;

// Биты статуса.
const ST_ERR: u8 = 0x01;
const ST_DRQ: u8 = 0x08;
const ST_DF: u8 = 0x20;
const ST_DRDY: u8 = 0x40;
const ST_BSY: u8 = 0x80;

// Биты в Drive/Head: 0xA0 | master/slave.
const DRIVE_MASTER: u8 = 0xA0;
const DRIVE_SLAVE: u8 = 0xB0;

// ATA команды.
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_READ_SECTORS_EXT: u8 = 0x24;
const CMD_IDENTIFY: u8 = 0xEC;
const CMD_IDENTIFY_PACKET: u8 = 0xA1;

/// Сколько итераций ждать готовности порта. PIO на медленном диске может
/// отвечать десятки миллисекунд; на 3 ГГц это миллионы циклов.
const IO_SPIN: u32 = 2_000_000;

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port,
        options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value,
        options(nostack, preserves_flags));
}

#[inline]
unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", out("ax") value, in("dx") port,
        options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value,
        options(nostack, preserves_flags));
}

/// Пауза 400 нс: четыре чтения alternate status (стандартный приём ATA).
#[inline]
fn io_delay(ctrl: u16) {
    unsafe {
        for _ in 0..4 {
            let _ = inb(ctrl + 2);
        }
    }
}

/// Канал: командный блок и блок управления.
#[derive(Clone, Copy)]
struct Channel {
    cmd: u16,
    ctrl: u16,
}

impl Channel {
    fn status(self) -> u8 {
        unsafe { inb(self.cmd + REG_STATUS) }
    }

    /// Ждать сброса BSY. `None` — таймаут (или устройство «висит»).
    fn wait_not_busy(self) -> Option<u8> {
        for _ in 0..IO_SPIN {
            let status = self.status();
            if status & ST_BSY == 0 {
                return Some(status);
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Ждать BSY=0 и DRQ=1.
    fn wait_drq(self) -> Option<u8> {
        for _ in 0..IO_SPIN {
            let status = self.status();
            if status & ST_BSY == 0 {
                if status & (ST_ERR | ST_DF) != 0 {
                    return None;
                }
                if status & ST_DRQ != 0 {
                    return Some(status);
                }
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Выбрать устройство и выдержать паузу.
    fn select(self, drive: u8) -> Option<u8> {
        unsafe {
            outb(self.cmd + REG_DRIVE, drive);
        }
        io_delay(self.ctrl);
        self.wait_not_busy()
    }
}

/// Паспорт найденного PATA-диска.
#[derive(Clone)]
pub struct IdeDiskInfo {
    pub channel: u8,
    pub drive: u8,
    pub model: String,
    pub serial: String,
    pub sectors: u64,
    pub sector_size: u16,
    pub lba48: bool,
}

static DISKS: Mutex<Vec<IdeDiskInfo>> = Mutex::new(Vec::new());

fn channels() -> [Channel; 2] {
    [
        Channel { cmd: PRIMARY_CMD, ctrl: PRIMARY_CTRL },
        Channel { cmd: SECONDARY_CMD, ctrl: SECONDARY_CTRL },
    ]
}

/// Порт вообще отвечает? На машинах без IDE контроллера читается 0xFF.
fn channel_present(channel: Channel) -> bool {
    channel.status() != 0xFF
}

/// IDENTIFY DEVICE / IDENTIFY PACKET DEVICE.
fn identify(channel: Channel, drive: u8) -> Option<([u16; 256], bool)> {
    channel.select(drive)?;

    unsafe {
        outb(channel.cmd + REG_COUNT, 0);
        outb(channel.cmd + REG_LBA0, 0);
        outb(channel.cmd + REG_LBA1, 0);
        outb(channel.cmd + REG_LBA2, 0);
        outb(channel.cmd + REG_COMMAND, CMD_IDENTIFY);
    }
    io_delay(channel.ctrl);

    // status == 0 означает «нет устройства» (шина не отвечает).
    let mut status = channel.status();
    if status == 0 {
        return None;
    }
    if status & ST_BSY != 0 {
        status = channel.wait_not_busy()?;
    }

    // ATAPI-устройство выставляет signature 0x14/0xEB и ошибку на IDENTIFY
    // DEVICE — повторяем с IDENTIFY PACKET DEVICE, чтобы хотя бы опознать его.
    if status & (ST_ERR | ST_DF) != 0 {
        let lba_mid = unsafe { inb(channel.cmd + REG_LBA1) };
        let lba_high = unsafe { inb(channel.cmd + REG_LBA2) };
        if lba_mid == 0x14 && lba_high == 0xEB {
            return identify_packet(channel, drive);
        }
        return None;
    }

    channel.wait_drq()?;
    Some((read_identify_words(channel), false))
}

fn identify_packet(channel: Channel, drive: u8) -> Option<([u16; 256], bool)> {
    channel.select(drive)?;
    unsafe {
        outb(channel.cmd + REG_COUNT, 0);
        outb(channel.cmd + REG_LBA0, 0);
        outb(channel.cmd + REG_LBA1, 0);
        outb(channel.cmd + REG_LBA2, 0);
        outb(channel.cmd + REG_COMMAND, CMD_IDENTIFY_PACKET);
    }
    io_delay(channel.ctrl);
    channel.wait_drq()?;
    Some((read_identify_words(channel), true))
}

fn read_identify_words(channel: Channel) -> [u16; 256] {
    let mut words = [0u16; 256];
    for slot in words.iter_mut() {
        *slot = unsafe { inw(channel.cmd + REG_DATA) };
    }
    words
}

fn ascii_field(words: &[u16; 256], from: usize, count: usize) -> String {
    let mut bytes = Vec::with_capacity(count * 2);
    for index in from..from + count {
        let word = words[index];
        bytes.push((word >> 8) as u8);
        bytes.push(word as u8);
    }
    while bytes.last() == Some(&b' ') || bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Опросить оба канала и оба устройства на каждом.
pub fn probe() -> usize {
    let mut found = Vec::new();

    for (channel_index, channel) in channels().iter().enumerate() {
        if !channel_present(*channel) {
            continue;
        }
        for (drive_index, drive) in [DRIVE_MASTER, DRIVE_SLAVE].iter().enumerate() {
            let Some((words, atapi)) = identify(*channel, *drive) else {
                continue;
            };
            if atapi {
                println!(
                    "[IDE] ch{} {}: ATAPI device (not supported by IDE driver)",
                    channel_index, drive_index
                );
                continue;
            }

            let mut sectors = (words[60] as u64) | ((words[61] as u64) << 16);
            let lba48 = words[83] & (1 << 10) != 0;
            if lba48 {
                let ext = (words[100] as u64)
                    | ((words[101] as u64) << 16)
                    | ((words[102] as u64) << 32)
                    | ((words[103] as u64) << 48);
                if ext != 0 {
                    sectors = ext;
                }
            }
            if sectors == 0 {
                println!(
                    "[IDE] ch{} {}: IDENTIFY returned 0 sectors (words 60={:#x} 61={:#x})",
                    channel_index, drive_index, words[60], words[61]
                );
                continue;
            }

            let sector_size = if words[106] & (1 << 12) != 0 {
                let size = (words[118] as u16) * 2;
                if size == 0 { 512 } else { size }
            } else {
                512
            };

            let info = IdeDiskInfo {
                channel: channel_index as u8,
                drive: *drive,
                model: ascii_field(&words, 27, 20),
                serial: ascii_field(&words, 10, 10),
                sectors,
                sector_size,
                lba48,
            };

            println!(
                "[IDE] ch{} {}: {} — {} sect ({} bytes/sect, {})",
                channel_index,
                drive_index,
                info.model,
                sectors,
                sector_size,
                if lba48 { "LBA48" } else { "LBA28" }
            );
            found.push(info);
        }
    }

    if found.is_empty() {
        // Частая причина «диски не видны»: SATA в BIOS переключён в IDE,
        // а драйвер AHCI такой контроллер не находит. Здесь же подсказываем.
        if crate::drivers::pci::find_device(0x01, 0x01).is_some() {
            println!("[IDE] controller present but no ATA disk answered IDENTIFY");
        }
        return 0;
    }
    let count = found.len();
    DISKS.lock().extend(found);
    count
}

pub fn disk_count() -> usize {
    DISKS.lock().len()
}

pub fn disks() -> Vec<IdeDiskInfo> {
    DISKS.lock().clone()
}

pub fn open(index: usize) -> Option<Box<dyn crate::fs::disk::DiskIo>> {
    let info = DISKS.lock().get(index)?.clone();
    Some(Box::new(IdeDisk { info, present: true }))
}

/// PATA-диск как `DiskIo`: 512-байтные сектора, только чтение.
pub struct IdeDisk {
    info: IdeDiskInfo,
    present: bool,
}

impl IdeDisk {
    fn channel(&self) -> Channel {
        channels()[self.info.channel as usize]
    }

    /// Одна PIO-команда чтения: `count` секторов в `buf`.
    fn read_pio(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        if !self.present {
            return Err(crate::fs::vfs::FsError::IoError);
        }
        if count == 0 || buf.len() < count as usize * 512 {
            return Err(crate::fs::vfs::FsError::InvalidPath);
        }

        let channel = self.channel();
        channel
            .select(self.info.drive)
            .ok_or(crate::fs::vfs::FsError::IoError)?;

        unsafe {
            if self.info.lba48 {
                outb(channel.cmd + REG_COUNT, (count >> 8) as u8);
                outb(channel.cmd + REG_LBA0, (lba >> 24) as u8);
                outb(channel.cmd + REG_LBA1, (lba >> 32) as u8);
                outb(channel.cmd + REG_LBA2, (lba >> 40) as u8);
                outb(channel.cmd + REG_COUNT, count as u8);
                outb(channel.cmd + REG_LBA0, lba as u8);
                outb(channel.cmd + REG_LBA1, (lba >> 8) as u8);
                outb(channel.cmd + REG_LBA2, (lba >> 16) as u8);
                outb(channel.cmd + REG_COMMAND, CMD_READ_SECTORS_EXT);
            } else {
                if lba > 0x0FFF_FFFF {
                    return Err(crate::fs::vfs::FsError::NotSupported);
                }
                outb(channel.cmd + REG_COUNT, count as u8);
                outb(channel.cmd + REG_LBA0, lba as u8);
                outb(channel.cmd + REG_LBA1, (lba >> 8) as u8);
                outb(channel.cmd + REG_LBA2, (lba >> 16) as u8);
                outb(
                    channel.cmd + REG_DRIVE,
                    self.info.drive | (((lba >> 24) & 0x0F) as u8),
                );
                outb(channel.cmd + REG_COMMAND, CMD_READ_SECTORS);
            }
        }
        io_delay(channel.ctrl);

        for sector in 0..count as usize {
            if channel.wait_drq().is_none() {
                self.present = false;
                return Err(crate::fs::vfs::FsError::IoError);
            }
            let base = sector * 512;
            for word in 0..256 {
                let value = unsafe { inw(channel.cmd + REG_DATA) };
                buf[base + word * 2] = value as u8;
                buf[base + word * 2 + 1] = (value >> 8) as u8;
            }
            io_delay(channel.ctrl);
        }

        Ok(())
    }
}

impl crate::fs::disk::DiskIo for IdeDisk {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), crate::fs::vfs::FsError> {
        if buf.is_empty() || buf.len() % 512 != 0 {
            return Err(crate::fs::vfs::FsError::InvalidPath);
        }
        let mut done = 0usize;
        while done < buf.len() {
            let remaining = (buf.len() - done) / 512;
            // Один PIO-приказ: не больше 32 секторов, чтобы не держать порт
            // занятым слишком долго и не терять отзывчивость.
            let count = remaining.min(32) as u16;
            let lba = sector + (done / 512) as u64;
            self.read_pio(lba, count, &mut buf[done..done + count as usize * 512])?;
            done += count as usize * 512;
        }
        Ok(())
    }

    fn sector_size(&self) -> usize {
        // PIO-чтение в этом драйвере всегда идёт 512-байтными секторами,
        // поэтому и block layer должен видеть 512: иначе GPT-парсер получит
        // 4096 и посчитает неверный LBA.
        512
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
