// procfs — виртуальная ФС для информации о системе

use super::vfs::*;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::boxed::Box;

pub struct ProcFs {
    mounted: bool,
}

impl ProcFs {
    pub fn new() -> Self {
        Self { mounted: false }
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &str { "procfs" }
    fn mount(&mut self) -> Result<(), FsError> { self.mounted = true; Ok(()) }
    fn unmount(&mut self) -> Result<(), FsError> { self.mounted = false; Ok(()) }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        // Нормализуем путь: убираем ведущий / если есть
        let path = path.trim_start_matches('/');
        let mut data: Vec<u8> = Vec::new();
        match path {
            "version" => data.extend_from_slice(b"Vibra OS 0.6 (Nucleus) kernel\n"),
            "meminfo" => data.extend_from_slice(b"MemTotal: 256 MB\nMemFree: 198 MB\n"),
            "cpuinfo" => data.extend_from_slice(b"processor: 0\nmodel name: QEMU Virtual CPU\ncpu MHz: 2400\n"),
            "uptime" => {
                let ticks = crate::interrupts::idt::ticks();
                let secs = ticks / 100;
                let mins = secs / 60;
                let hours = mins / 60;
                let s = alloc::format!("{}h {}m {}s\n", hours, mins % 60, secs % 60);
                data.extend_from_slice(s.as_bytes());
            }
            "modules" => data.extend_from_slice(b"vibra_core\nramfs\n"),
            _ => return Err(FsError::NotFound),
        }
        Ok(Box::new(ProcFile { data, pos: 0 }))
    }

    fn create(&mut self, _: &str) -> Result<Box<dyn File>, FsError> { Err(FsError::ReadOnly) }
    fn remove(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn mkdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }
    fn rmdir(&mut self, _: &str) -> Result<(), FsError> { Err(FsError::ReadOnly) }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = path.trim_start_matches('/');
        match path {
            "" | "version" | "meminfo" | "cpuinfo" | "uptime" | "modules" => {
                // Если путь указывает на файл — возвращаем пустой список (не директория)
                if !path.is_empty() { return Ok(Vec::new()); }
                Ok(alloc::vec![
                    DirEntry { name: String::from("version"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                    DirEntry { name: String::from("meminfo"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                    DirEntry { name: String::from("cpuinfo"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                    DirEntry { name: String::from("uptime"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                    DirEntry { name: String::from("modules"), file_type: FileType::File, size: 0, permissions: Permissions::new(0o444), uid: 0, gid: 0 },
                ])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn exists(&self, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        matches!(path, "version" | "meminfo" | "cpuinfo" | "uptime" | "modules")
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        Ok(FileMetadata {
            name: String::from(path),
            file_type: FileType::File,
            size: 0,
            permissions: Permissions::new(0o444),
            uid: 0, gid: 0, created: 0, modified: 0,
        })
    }
}

struct ProcFile { data: Vec<u8>, pos: u64 }

impl File for ProcFile {
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
