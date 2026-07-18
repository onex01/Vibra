use super::vfs::{FsError, FileSystem, File, FileMetadata, DirEntry};
use alloc::vec::Vec;
use alloc::boxed::Box;

pub struct Superblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_cluster_size: u32,
    pub blocks_per_group: u32,
    pub clusters_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
}

impl Superblock {
    pub fn parse(data: &[u8]) -> Result<Self, FsError> {
        if data.len() < 1024 {
            return Err(FsError::IoError);
        }
        
        let magic = u16::from_le_bytes([data[1080], data[1081]]);
        if magic != 0xEF53 {
            return Err(FsError::IoError);
        }
        
        Ok(Superblock {
            inodes_count: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            blocks_count: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            r_blocks_count: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            free_blocks_count: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            free_inodes_count: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            first_data_block: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            log_block_size: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            log_cluster_size: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
            blocks_per_group: u32::from_le_bytes([data[32], data[33], data[34], data[35]]),
            clusters_per_group: u32::from_le_bytes([data[36], data[37], data[38], data[39]]),
            inodes_per_group: u32::from_le_bytes([data[40], data[41], data[42], data[43]]),
            mtime: u32::from_le_bytes([data[44], data[45], data[46], data[47]]),
            wtime: u32::from_le_bytes([data[48], data[49], data[50], data[51]]),
            magic: u16::from_le_bytes([data[1080], data[1081]]),
            state: u16::from_le_bytes([data[1084], data[1085]]),
            errors: u16::from_le_bytes([data[1086], data[1087]]),
            minor_rev_level: u16::from_le_bytes([data[1094], data[1095]]),
        })
    }
}

pub struct Ext2Fs {
    pub(super) superblock: Superblock,
    block_size: u32,
    mounted: bool,
}

impl Ext2Fs {
    pub fn new() -> Result<Self, FsError> {
        Ok(Self {
            superblock: Superblock {
                inodes_count: 0,
                blocks_count: 0,
                r_blocks_count: 0,
                free_blocks_count: 0,
                free_inodes_count: 0,
                first_data_block: 0,
                log_block_size: 0,
                log_cluster_size: 0,
                blocks_per_group: 0,
                clusters_per_group: 0,
                inodes_per_group: 0,
                mtime: 0,
                wtime: 0,
                magic: 0,
                state: 0,
                errors: 0,
                minor_rev_level: 0,
            },
            block_size: 1024,
            mounted: false,
        })
    }
}

impl FileSystem for Ext2Fs {
    fn name(&self) -> &str {
        "ext2"
    }
    
    fn mount(&mut self) -> Result<(), FsError> {
        self.mounted = true;
        Ok(())
    }
    
    fn unmount(&mut self) -> Result<(), FsError> {
        self.mounted = false;
        Ok(())
    }
    
    fn open(&self, _path: &str) -> Result<Box<dyn File>, FsError> {
        Err(FsError::NotFound)
    }
    
    fn create(&mut self, _path: &str) -> Result<Box<dyn File>, FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn remove(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn mkdir(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn rmdir(&mut self, _path: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
    
    fn readdir(&mut self, _path: &str) -> Result<Vec<DirEntry>, FsError> {
        // EXT2 реализация ещё не завершена
        Ok(Vec::new())
    }
    
    fn exists(&self, _path: &str) -> bool {
        false
    }
    
    fn stat(&self, _path: &str) -> Result<FileMetadata, FsError> {
        Err(FsError::NotFound)
    }
}