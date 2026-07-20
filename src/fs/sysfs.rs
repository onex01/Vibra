// sysfs — виртуальная ФС для информации об устройстве

use super::vfs::*;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::boxed::Box;

pub struct SysFs { mounted: bool }

impl SysFs {
    pub fn new() -> Self { Self { mounted: false } }
}

impl FileSystem for SysFs {
    fn name(&self) -> &str { "sysfs" }
    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let path = path.trim_start_matches('/');
        let mut data: Vec<u8> = Vec::new();
        match path {
            "kernel/version" => data.extend_from_slice(b"0.6.4\n"),
            "kernel/name" => data.extend_from_slice(b"Vibra\n"),
            "kernel/arch" => data.extend_from_slice(b"x86_64\n"),
            "devices/count" => data.extend_from_slice(b"5\n"),
            "modules/count" => data.extend_from_slice(b"3\n"),
            _ => return Err(FsError::NotFound),
        }
        Ok(Box::new(SysFile { data, pos: 0 }))
    }

    fn create(&mut self, _: &str) -> Result<Box<dyn File>, FsError> { Err(FsError::ReadOnly) }
    fn remove(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = path.trim_start_matches('/');
        match path {
            "" => Ok(alloc::vec![
                DirEntry { name: String::from("kernel"), file_type: FileType::Directory, size: 0, permissions: Permissions::new(0o555), uid: 0, gid: 0 },
                DirEntry { name: String::from("devices"), file_type: FileType::Directory, size: 0, permissions: Permissions::new(0o555), uid: 0, gid: 0 },
                DirEntry { name: String::from("modules"), file_type: FileType::Directory, size: 0, permissions: Permissions::new(0o555), uid: 0, gid: 0 },
            ]),
            "kernel" => Ok(alloc::vec![
                DirEntry { name: String::from("version"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                DirEntry { name: String::from("name"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                DirEntry { name: String::from("arch"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
            ]),
            _ => Ok(Vec::new()),
        }
    }

    fn exists(&self, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        matches!(path, "kernel/version" | "kernel/name" | "kernel/arch" | "devices/count" | "modules/count")
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        Ok(FileMetadata { name: String::from(path), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0, created: 0, modified: 0 })
    }
}

struct SysFile { data: Vec<u8>, pos: u64 }
impl File for SysFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let p = self.pos as usize;
        if p >= self.data.len() { return Ok(0); }
        let n = buf.len().min(self.data.len() - p);
        buf[..n].copy_from_slice(&self.data[p..p + n]);
        self.pos += n as u64;
        Ok(n)
    }
    fn write(&mut self, _: &[u8]) -> Result<usize, FsError> { Err(FsError::ReadOnly) }
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos { SeekFrom::Start(o) => o as i64, SeekFrom::Current(o) => self.pos as i64 + o, SeekFrom::End(o) => self.data.len() as i64 + o };
        if new < 0 { return Err(FsError::InvalidPath); }
        self.pos = new as u64; Ok(self.pos)
    }
    fn position(&self) -> u64 { self.pos }
    fn size(&self) -> usize { self.data.len() }
}
