// devtmpfs — виртуальная ФС для устройств

use super::vfs::*;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::boxed::Box;

pub struct DevTmpFs { mounted: bool }

impl DevTmpFs {
    pub fn new() -> Self { Self { mounted: false } }
}

impl FileSystem for DevTmpFs {
    fn name(&self) -> &str { "devtmpfs" }
    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        match path {
            "/null" => Ok(Box::new(NullDev)),
            "/zero" => Ok(Box::new(ZeroDev)),
            "/random" | "/urandom" => Ok(Box::new(RandomDev)),
            _ => Err(FsError::NotFound),
        }
    }

    fn create(&mut self, _: &str) -> Result<Box<dyn File>, FsError> { Err(FsError::ReadOnly) }
    fn remove(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            Ok(alloc::vec![
                DirEntry { name: String::from("null"), file_type: FileType::Device, size: 0, permissions: Permissions::new(0o666), uid: 0, gid: 0 },
                DirEntry { name: String::from("zero"), file_type: FileType::Device, size: 0, permissions: Permissions::new(0o666), uid: 0, gid: 0 },
                DirEntry { name: String::from("random"), file_type: FileType::Device, size: 0, permissions: Permissions::new(0o666), uid: 0, gid: 0 },
                DirEntry { name: String::from("urandom"), file_type: FileType::Device, size: 0, permissions: Permissions::new(0o666), uid: 0, gid: 0 },
            ])
        } else {
            Ok(Vec::new())
        }
    }

    fn exists(&self, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        matches!(path, "null" | "zero" | "random" | "urandom")
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        Ok(FileMetadata { name: String::from(path), file_type: FileType::Device, size: 0, permissions: Permissions::new(0o666), uid: 0, gid: 0, created: 0, modified: 0 })
    }
}

// Устройства

struct NullDev;
impl File for NullDev {
    fn read(&mut self, _: &mut [u8]) -> Result<usize, FsError> { Ok(0) }
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> { Ok(buf.len()) }
    fn seek(&mut self, _: SeekFrom) -> Result<u64, FsError> { Ok(0) }
    fn position(&self) -> u64 { 0 }
    fn size(&self) -> usize { 0 }
}

struct ZeroDev;
impl File for ZeroDev {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        for b in buf.iter_mut() { *b = 0; }
        Ok(buf.len())
    }
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> { Ok(buf.len()) }
    fn seek(&mut self, _: SeekFrom) -> Result<u64, FsError> { Ok(0) }
    fn position(&self) -> u64 { 0 }
    fn size(&self) -> usize { 0 }
}

struct RandomDev;
impl File for RandomDev {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(0x9E) ^ 0x55;
        }
        Ok(buf.len())
    }
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> { Ok(buf.len()) }
    fn seek(&mut self, _: SeekFrom) -> Result<u64, FsError> { Ok(0) }
    fn position(&self) -> u64 { 0 }
    fn size(&self) -> usize { 0 }
}
