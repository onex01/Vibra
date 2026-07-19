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
    pub magic: u16,
}

impl Ext2Superblock {
    fn parse(data: &[u8]) -> Result<Self, FsError> {
        if data.len() < 1024 { return Err(FsError::IoError); }
        let magic = u16::from_le_bytes([data[1080], data[1081]]);
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
            magic,
        })
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
        let mut buf = [0u8; 1024];
        disk.read(1, &mut buf).map_err(|_| FsError::IoError)?;
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

    fn read_inode_data(&self, inode_num: u32) -> Result<Vec<u8>, FsError> {
        let group = ((inode_num - 1) / self.superblock.inodes_per_group) as usize;
        let index = ((inode_num - 1) % self.superblock.inodes_per_group) as usize;
        let inode_size = 128usize;
        let inodes_per_block = self.block_size as usize / inode_size;

        // Block containing the inode table for this group
        let inode_table_block = self.superblock.first_data_block + 1 + (group as u32) * self.superblock.blocks_per_group / 32;

        let mut block_buf = vec![0u8; self.block_size as usize];
        self.read_block(inode_table_block, &mut block_buf)?;

        let offset = (index % inodes_per_block) * inode_size;
        Ok(block_buf[offset..offset + inode_size].to_vec())
    }

    fn read_inode_block(&self, inode_num: u32) -> Result<u32, FsError> {
        let data = self.read_inode_data(inode_num)?;
        // First direct block is at offset 40 in inode structure
        Ok(u32::from_le_bytes([data[40], data[41], data[42], data[43]]))
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

    fn read_dir_entries(&self, inode_num: u32) -> Result<Vec<(String, bool)>, FsError> {
        let root_block = self.read_inode_block(inode_num)?;
        let size = self.get_inode_size(inode_num)?;

        let mut data = vec![0u8; size];
        let mut offset = 0;
        let mut current_block = root_block;

        while offset < size && current_block != 0 {
            let mut block_buf = vec![0u8; self.block_size as usize];
            self.read_block(current_block, &mut block_buf)?;
            let to_copy = (size - offset).min(self.block_size as usize);
            data[offset..offset + to_copy].copy_from_slice(&block_buf[..to_copy]);
            offset += to_copy;
            // For simplicity, just read first block
            break;
        }

        let mut entries = Vec::new();
        let mut pos = 0;
        while pos + 8 <= data.len() {
            let inode = u32::from_le_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]);
            let rec_len = u16::from_le_bytes([data[pos+4], data[pos+5]]) as usize;
            let name_len = data[pos+6] as usize;
            let file_type = data[pos+7];

            if inode == 0 || rec_len == 0 { break; }

            if name_len > 0 && name_len <= 255 {
                let name = core::str::from_utf8(&data[pos+8..pos+8+name_len])
                    .unwrap_or("")
                    .to_string();
                let is_dir = file_type == 2;
                entries.push((name, is_dir));
            }

            pos += rec_len;
        }

        Ok(entries)
    }

    fn find_in_dir(&self, dir_inode: u32, name: &str) -> Result<u32, FsError> {
        let entries = self.read_dir_entries(dir_inode)?;
        for (entry_name, _) in &entries {
            if entry_name.eq_ignore_ascii_case(name) {
                // Find the inode number for this entry
                let root_block = self.read_inode_block(dir_inode)?;
                let size = self.get_inode_size(dir_inode)?;
                let mut block_buf = vec![0u8; self.block_size as usize];
                self.read_block(root_block, &mut block_buf)?;

                let mut pos = 0;
                while pos + 8 <= size.min(block_buf.len()) {
                    let inode_num = u32::from_le_bytes([
                        block_buf[pos], block_buf[pos+1],
                        block_buf[pos+2], block_buf[pos+3],
                    ]);
                    let rec_len = u16::from_le_bytes([block_buf[pos+4], block_buf[pos+5]]) as usize;
                    let name_len = block_buf[pos+6] as usize;

                    if name_len > 0 && name_len <= 255 {
                        let entry_name = core::str::from_utf8(&block_buf[pos+8..pos+8+name_len])
                            .unwrap_or("");
                        if entry_name.eq_ignore_ascii_case(name) {
                            return Ok(inode_num);
                        }
                    }
                    pos += rec_len;
                }
                return Err(FsError::NotFound);
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

        let size = self.get_inode_size(inode_num)?;
        let root_block = self.read_inode_block(inode_num)?;

        let mut data = vec![0u8; size];
        if size > 0 && root_block != 0 {
            let mut block_buf = vec![0u8; self.block_size as usize];
            self.read_block(root_block, &mut block_buf)?;
            let to_copy = size.min(self.block_size as usize);
            data[..to_copy].copy_from_slice(&block_buf[..to_copy]);
        }

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

        for (name, is_dir) in &entries {
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
