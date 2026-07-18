/// Тип файлового объекта
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    File,
    Directory,
}

/// Метаданные файла
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub name: [u8; 32],
    pub name_len: usize,
    pub file_type: FileType,
    pub size: usize,
    pub created: u64,
    pub parent: [u8; 32],
    pub parent_len: usize,
}

/// Запись в файловой системе
#[derive(Clone)]
pub struct FsEntry {
    pub metadata: FileMetadata,
    pub data: [u8; 4096],
    pub data_len: usize,
}

impl FsEntry {
    pub fn new_file(name: &str) -> Self {
        let mut entry = FsEntry {
            metadata: FileMetadata {
                name: [0u8; 32],
                name_len: 0,
                file_type: FileType::File,
                size: 0,
                created: 0,
                parent: [0u8; 32],
                parent_len: 0,
            },
            data: [0u8; 4096],
            data_len: 0,
        };
        entry.set_name(name);
        entry
    }

    pub fn new_dir(name: &str) -> Self {
        let mut entry = FsEntry {
            metadata: FileMetadata {
                name: [0u8; 32],
                name_len: 0,
                file_type: FileType::Directory,
                size: 0,
                created: 0,
                parent: [0u8; 32],
                parent_len: 0,
            },
            data: [0u8; 4096],
            data_len: 0,
        };
        entry.set_name(name);
        entry
    }

    fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.metadata.name[..len].copy_from_slice(&bytes[..len]);
        self.metadata.name_len = len;
    }

    pub fn set_parent(&mut self, parent: &str) {
        let bytes = parent.as_bytes();
        let len = bytes.len().min(31);
        self.metadata.parent[..len].copy_from_slice(&bytes[..len]);
        self.metadata.parent_len = len;
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.metadata.name[..self.metadata.name_len]).unwrap_or("?")
    }

    pub fn parent(&self) -> &str {
        core::str::from_utf8(&self.metadata.parent[..self.metadata.parent_len]).unwrap_or("")
    }
}

/// Простая файловая система в памяти
pub struct RamFs {
    entries: [Option<FsEntry>; 64],
    count: usize,
}

impl RamFs {
    pub const fn new() -> Self {
        const NONE: Option<FsEntry> = None;
        RamFs {
            entries: [NONE; 64],
            count: 0,
        }
    }

    pub fn create_file(&mut self, name: &str) -> Result<(), &'static str> {
        if self.count >= 64 {
            return Err("File system full");
        }

        for i in 0..64 {
            if let Some(entry) = &self.entries[i] {
                if entry.name() == name {
                    return Err("File already exists");
                }
            }
        }

        for i in 0..64 {
            if self.entries[i].is_none() {
                self.entries[i] = Some(FsEntry::new_file(name));
                self.count += 1;
                return Ok(());
            }
        }

        Err("No free slots")
    }

    pub fn create_dir(&mut self, name: &str) -> Result<(), &'static str> {
        if self.count >= 64 {
            return Err("File system full");
        }

        for i in 0..64 {
            if let Some(entry) = &self.entries[i] {
                if entry.name() == name {
                    return Err("Directory already exists");
                }
            }
        }

        for i in 0..64 {
            if self.entries[i].is_none() {
                self.entries[i] = Some(FsEntry::new_dir(name));
                self.count += 1;
                return Ok(());
            }
        }

        Err("No free slots")
    }

    pub fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), &'static str> {
        for i in 0..64 {
            if let Some(entry) = &mut self.entries[i] {
                if entry.name() == name && entry.metadata.file_type == FileType::File {
                    let len = data.len().min(4096);
                    entry.data[..len].copy_from_slice(&data[..len]);
                    entry.data_len = len;
                    entry.metadata.size = len;
                    return Ok(());
                }
            }
        }
        Err("File not found")
    }

    pub fn read_file(&self, name: &str) -> Result<&[u8], &'static str> {
        for i in 0..64 {
            if let Some(entry) = &self.entries[i] {
                if entry.name() == name && entry.metadata.file_type == FileType::File {
                    return Ok(&entry.data[..entry.data_len]);
                }
            }
        }
        Err("File not found")
    }

    pub fn remove(&mut self, name: &str) -> Result<(), &'static str> {
        for i in 0..64 {
            if let Some(entry) = &self.entries[i] {
                if entry.name() == name {
                    self.entries[i] = None;
                    self.count -= 1;
                    return Ok(());
                }
            }
        }
        Err("Entry not found")
    }

    pub fn list(&self) -> impl Iterator<Item = &FsEntry> {
        self.entries.iter().filter_map(|e| e.as_ref())
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

static mut FILESYSTEM: RamFs = RamFs::new();

pub fn init_filesystem() {
    unsafe {
        FILESYSTEM.create_file("readme.txt").ok();
        FILESYSTEM.write_file("readme.txt", b"Welcome to Vibra OS!\nThis is a simple text file.").ok();

        FILESYSTEM.create_file("version.txt").ok();
        FILESYSTEM.write_file("version.txt", b"Vibra OS 0.4 Photon\nKernel 0.4.0").ok();

        FILESYSTEM.create_file("about.txt").ok();
        FILESYSTEM.write_file("about.txt", b"Vibra OS\n========\n\nCreated by: OneX01\nDate: 2026-07-18\nLicense: MIT\n\nA hobby OS written in Rust.").ok();

        FILESYSTEM.create_dir("docs").ok();
        FILESYSTEM.create_dir("home").ok();
    }
}

pub fn list_entries() -> impl Iterator<Item = &'static FsEntry> {
    unsafe { FILESYSTEM.list() }
}

pub fn create_file(name: &str) -> Result<(), &'static str> {
    unsafe { FILESYSTEM.create_file(name) }
}

pub fn create_dir(name: &str) -> Result<(), &'static str> {
    unsafe { FILESYSTEM.create_dir(name) }
}

pub fn write_file(name: &str, data: &[u8]) -> Result<(), &'static str> {
    unsafe { FILESYSTEM.write_file(name, data) }
}

pub fn read_file(name: &str) -> Result<&'static [u8], &'static str> {
    unsafe { FILESYSTEM.read_file(name) }
}

pub fn remove_entry(name: &str) -> Result<(), &'static str> {
    unsafe { FILESYSTEM.remove(name) }
}

pub fn fs_count() -> usize {
    unsafe { FILESYSTEM.count() }
}