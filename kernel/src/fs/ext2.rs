// EXT2 File System Driver — базовая реализация чтения

use super::vfs::*;
use super::disk::DiskIo;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::boxed::Box;
use alloc::vec;

const EXT2_SUPER_MAGIC: u16 = 0xEF53;
const SECTOR_SIZE: usize = 512;

pub struct Ext2Superblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub rev_level: u32,
    pub inode_size: u16,
    pub magic: u16,
}

impl Ext2Superblock {
    /// Разбор 1024-байтного суперблока (смещения — от его начала).
    fn parse(data: &[u8]) -> Result<Self, FsError> {
        if data.len() < 1024 { return Err(FsError::IoError); }
        let magic = u16::from_le_bytes([data[56], data[57]]);
        if magic != EXT2_SUPER_MAGIC { return Err(FsError::InvalidPath); }

        Ok(Self {
            inodes_count: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            blocks_count: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            r_blocks_count: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            free_blocks_count: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            free_inodes_count: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            first_data_block: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            log_block_size: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            blocks_per_group: u32::from_le_bytes([data[32], data[33], data[34], data[35]]),
            inodes_per_group: u32::from_le_bytes([data[40], data[41], data[42], data[43]]),
            // s_rev_level (0x4C) и s_inode_size (0x58): у ext2 rev0 размер inode
            // всегда 128 байт, у rev1 — из суперблока (обычно 128 или 256).
            rev_level: u32::from_le_bytes([data[76], data[77], data[78], data[79]]),
            inode_size: u16::from_le_bytes([data[88], data[89]]),
            magic,
        })
    }

    /// Реальный размер inode в байтах.
    fn inode_size_bytes(&self) -> usize {
        if self.rev_level >= 1 && self.inode_size >= 128 {
            self.inode_size as usize
        } else {
            128
        }
    }
}

pub struct Ext2Fs {
    disk: spin::Mutex<Box<dyn DiskIo>>,
    superblock: Ext2Superblock,
    block_size: u32,
    mounted: bool,
}

impl Ext2Fs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        // Суперблок ext2 всегда лежит по смещению 1024 байта (сектор 2 при
        // 512-байтных секторах). Раньше читался сектор 1, из-за чего магия
        // 0xEF53 искалась на 512 байт раньше и монтирование всегда падало.
        let mut buf = [0u8; 1024];
        disk.read(2, &mut buf).map_err(|_| FsError::IoError)?;
        let superblock = Ext2Superblock::parse(&buf)?;
        let block_size = 1024 << superblock.log_block_size;

        Ok(Self {
            disk: spin::Mutex::new(disk),
            superblock,
            block_size,
            mounted: false,
        })
    }

    fn read_block(&self, block: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let sector = block as u64 * (self.block_size as u64 / SECTOR_SIZE as u64);
        self.disk.lock().read(sector, buf).map_err(|_| FsError::IoError)
    }

    /// Блок таблицы inode для группы `group`.
    ///
    /// Адрес берётся из Group Descriptor (bg_inode_table, смещение 8 в дескрипторе).
    /// Раньше здесь стояло `first_data_block + 1 + group * blocks_per_group / 32` —
    /// это не имеет отношения к раскладке ext2 и давало мусор для любой группы > 0.
    fn inode_table_block(&self, group: u32) -> Result<u32, FsError> {
        let gdt_block = self.superblock.first_data_block + 1;
        let desc_size = 32usize;
        let descs_per_block = (self.block_size as usize / desc_size).max(1);
        let block_for_group = gdt_block + group / descs_per_block as u32;
        let index_in_block = (group as usize % descs_per_block) * desc_size;

        let mut buf = vec![0u8; self.block_size as usize];
        self.read_block(block_for_group, &mut buf)?;
        let table = u32::from_le_bytes([
            buf[index_in_block + 8],
            buf[index_in_block + 9],
            buf[index_in_block + 10],
            buf[index_in_block + 11],
        ]);
        if table == 0 { return Err(FsError::IoError); }
        Ok(table)
    }

    fn read_inode_data(&self, inode_num: u32) -> Result<Vec<u8>, FsError> {
        if inode_num == 0 { return Err(FsError::NotFound); }
        let per_group = self.superblock.inodes_per_group.max(1);
        let group = (inode_num - 1) / per_group;
        let index = (inode_num - 1) % per_group;

        let inode_size = self.superblock.inode_size_bytes();
        let inodes_per_block = (self.block_size as usize / inode_size).max(1);

        let table = self.inode_table_block(group)?;
        let block = table + (index as usize / inodes_per_block) as u32;
        let offset = (index as usize % inodes_per_block) * inode_size;

        let mut buf = vec![0u8; self.block_size as usize];
        self.read_block(block, &mut buf)?;
        Ok(buf[offset..offset + inode_size].to_vec())
    }

    /// Все блоки файла/каталога по порядку: 12 прямых + single/double indirect.
    ///
    /// Нулевой элемент означает «дыру» (sparse-файл): такой блок не читается,
    /// но позиция в выходном буфере под него сохраняется.
    fn inode_blocks(&self, inode_num: u32, size: usize) -> Result<Vec<u32>, FsError> {
        let inode = self.read_inode_data(inode_num)?;
        let mut blocks: Vec<u32> = Vec::new();
        let needed = (size + self.block_size as usize - 1) / self.block_size as usize;
        if needed == 0 { return Ok(blocks); }

        let ptr = |i: usize| -> u32 {
            let o = 40 + i * 4;
            u32::from_le_bytes([inode[o], inode[o + 1], inode[o + 2], inode[o + 3]])
        };
        let per_block = (self.block_size as usize / 4).max(1);
        let mut buf = vec![0u8; self.block_size as usize];
        let read_ptr = |buf: &[u8], i: usize| -> u32 {
            u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]])
        };

        // 12 прямых ссылок (0 = дыра, но место занимает).
        for i in 0..12 {
            if blocks.len() >= needed { return Ok(blocks); }
            blocks.push(ptr(i));
        }

        // Одиночная косвенная (i_block[12]).
        if blocks.len() < needed {
            let indirect = ptr(12);
            if indirect == 0 {
                for _ in 0..per_block {
                    if blocks.len() >= needed { return Ok(blocks); }
                    blocks.push(0);
                }
            } else {
                self.read_block(indirect, &mut buf)?;
                for i in 0..per_block {
                    if blocks.len() >= needed { return Ok(blocks); }
                    blocks.push(read_ptr(&buf, i));
                }
            }
        }

        // Двойная косвенная (i_block[13]).
        if blocks.len() < needed {
            let dindirect = ptr(13);
            if dindirect == 0 {
                for _ in 0..(per_block * per_block) {
                    if blocks.len() >= needed { return Ok(blocks); }
                    blocks.push(0);
                }
            } else {
                let mut l1 = vec![0u8; self.block_size as usize];
                self.read_block(dindirect, &mut l1)?;
                for i in 0..per_block {
                    if blocks.len() >= needed { return Ok(blocks); }
                    let second = read_ptr(&l1, i);
                    if second == 0 {
                        for _ in 0..per_block {
                            if blocks.len() >= needed { return Ok(blocks); }
                            blocks.push(0);
                        }
                        continue;
                    }
                    self.read_block(second, &mut buf)?;
                    for j in 0..per_block {
                        if blocks.len() >= needed { return Ok(blocks); }
                        blocks.push(read_ptr(&buf, j));
                    }
                }
            }
        }

        Ok(blocks)
    }

    /// Содержимое файла/каталога целиком (без чтения за пределами размера inode).
    fn read_inode_content(&self, inode_num: u32) -> Result<Vec<u8>, FsError> {
        let size = self.get_inode_size(inode_num)?;
        if size == 0 { return Ok(Vec::new()); }

        let blocks = self.inode_blocks(inode_num, size)?;
        let bs = self.block_size as usize;
        let mut data = vec![0u8; size];
        let mut buf = vec![0u8; bs];

        for (i, b) in blocks.iter().enumerate() {
            let off = i * bs;
            if off >= size { break; }
            if *b == 0 { continue; } // дыра — остаётся нулевой
            self.read_block(*b, &mut buf)?;
            let take = (size - off).min(bs);
            data[off..off + take].copy_from_slice(&buf[..take]);
        }

        Ok(data)
    }

    fn get_inode_size(&self, inode_num: u32) -> Result<usize, FsError> {
        let data = self.read_inode_data(inode_num)?;
        // Size is at offset 4 in inode structure
        Ok(u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize)
    }

    fn is_inode_dir(&self, inode_num: u32) -> Result<bool, FsError> {
        let data = self.read_inode_data(inode_num)?;
        // Mode is at offset 0 in inode structure
        let mode = u16::from_le_bytes([data[0], data[1]]);
        Ok((mode & 0x4000) != 0)
    }

    /// Записи каталога: (имя, номер inode, это_каталог).
    /// Читается ВСЁ содержимое каталога, а не только первый блок.
    fn read_dir_entries(&self, inode_num: u32) -> Result<Vec<(String, u32, bool)>, FsError> {
        let data = self.read_inode_content(inode_num)?;
        let mut entries = Vec::new();
        let mut pos = 0usize;

        while pos + 8 <= data.len() {
            let inode = u32::from_le_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]);
            let rec_len = u16::from_le_bytes([data[pos+4], data[pos+5]]) as usize;
            let name_len = data[pos+6] as usize;
            let file_type = data[pos+7];

            if rec_len < 8 || pos + rec_len > data.len() { break; }

            if inode != 0 && name_len > 0 && name_len <= 255 && pos + 8 + name_len <= data.len() {
                let name = core::str::from_utf8(&data[pos+8..pos+8+name_len]).unwrap_or("");
                if !name.is_empty() {
                    // file_type: 1 = regular, 2 = dir; 0 = «неизвестно» (старый ext2).
                    let is_dir = if file_type == 0 {
                        self.is_inode_dir(inode).unwrap_or(false)
                    } else {
                        file_type == 2
                    };
                    entries.push((name.to_string(), inode, is_dir));
                }
            }

            pos += rec_len;
        }

        Ok(entries)
    }

    fn find_in_dir(&self, dir_inode: u32, name: &str) -> Result<u32, FsError> {
        for (entry_name, inode, _) in self.read_dir_entries(dir_inode)? {
            if entry_name.eq_ignore_ascii_case(name) {
                return Ok(inode);
            }
        }
        Err(FsError::NotFound)
    }

    fn find_path(&self, path: &str) -> Result<u32, FsError> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() { return Ok(2); }

        let mut current_inode = 2u32;
        for part in &parts {
            current_inode = self.find_in_dir(current_inode, part)?;
        }
        Ok(current_inode)
    }
}

pub struct Ext2File {
    data: Vec<u8>,
    position: u64,
}

impl File for Ext2File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() { return Ok(0); }
        let to_read = buf.len().min(self.data.len() - pos);
        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }
    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> { Err(FsError::ReadOnly) }
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::Current(o) => self.position as i64 + o,
            SeekFrom::End(o) => self.data.len() as i64 + o,
        };
        if new < 0 { return Err(FsError::InvalidPath); }
        self.position = new as u64;
        Ok(self.position)
    }
    fn position(&self) -> u64 { self.position }
    fn size(&self) -> usize { self.data.len() }
}

impl FileSystem for Ext2Fs {
    fn name(&self) -> &str { "ext2" }
    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let inode_num = self.find_path(path)?;
        if self.is_inode_dir(inode_num)? {
            return Err(FsError::IsADirectory);
        }

        let data = self.read_inode_content(inode_num)?;

        Ok(Box::new(Ext2File { data, position: 0 }))
    }

    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> { Err(FsError::ReadOnly) }
    fn remove(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _path: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let inode_num = self.find_path(path)?;
        if !self.is_inode_dir(inode_num)? {
            return Err(FsError::NotADirectory);
        }

        let entries = self.read_dir_entries(inode_num)?;
        let mut result = Vec::new();

        for (name, _inode, is_dir) in &entries {
            if name == "." || name == ".." { continue; }
            let file_type = if *is_dir { FileType::Directory } else { FileType::File };
            let perms = if *is_dir { 0o755 } else { 0o644 };

            result.push(DirEntry {
                name: name.clone(),
                file_type,
                size: 0,
                permissions: Permissions::new(perms),
                uid: 0,
                gid: 0,
            });
        }
        Ok(result)
    }

    fn exists(&self, path: &str) -> bool {
        self.find_path(path).is_ok()
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let inode_num = self.find_path(path)?;
        let file_type = if self.is_inode_dir(inode_num)? { FileType::Directory } else { FileType::File };
        let perms = if file_type == FileType::Directory { 0o755 } else { 0o644 };
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let name = parts.last().unwrap_or(&"/").to_string();

        Ok(FileMetadata {
            name, file_type, size: self.get_inode_size(inode_num)?,
            permissions: Permissions::new(perms),
            uid: 0, gid: 0, created: 0, modified: 0,
        })
    }
}
