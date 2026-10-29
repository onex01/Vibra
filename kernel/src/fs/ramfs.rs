// RamFS — файловая система в памяти.
//
// Одна и та же ФС должна быть видна и через VFS (mount "/"), и через legacy-API
// (`fs::write_file`, `hostname`, `/var/log/boot.log`). Поэтому состояние живёт за
// `Arc<Mutex<RamFsInner>>`, а открытый файл хранит путь и тот же `Arc` — запись
// через handle попадает в каталог, а не в отдельную копию `Vec<u8>`.

use super::vfs::*;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use spin::Mutex;

/// Внутреннее представление записи.
#[derive(Clone)]
struct RamFsEntry {
    name: String,
    file_type: FileType,
    data: Vec<u8>,
    created: u64,
    modified: u64,
}

impl RamFsEntry {
    fn new_file(name: &str) -> Self {
        RamFsEntry {
            name: String::from(name),
            file_type: FileType::File,
            data: Vec::new(),
            created: 0,
            modified: 0,
        }
    }

    fn new_dir(name: &str) -> Self {
        RamFsEntry {
            name: String::from(name),
            file_type: FileType::Directory,
            data: Vec::new(),
            created: 0,
            modified: 0,
        }
    }
}

struct RamFsInner {
    entries: Vec<RamFsEntry>,
    mounted: bool,
}

/// Файловая система в памяти. Клон — дешёвая копия `Arc`, состояние общее.
#[derive(Clone)]
pub struct RamFs {
    inner: Arc<Mutex<RamFsInner>>,
}

impl RamFs {
    pub fn new() -> Self {
        RamFs {
            inner: Arc::new(Mutex::new(RamFsInner {
                entries: Vec::new(),
                mounted: false,
            })),
        }
    }

    /// Записать данные в существующий файл напрямую (для init).
    pub fn write_data(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let path = normalize_path(path);
        let mut inner = self.inner.lock();

        for entry in &mut inner.entries {
            if entry.name == path && entry.file_type == FileType::File {
                entry.data.clear();
                entry.data.extend_from_slice(data);
                return Ok(());
            }
        }
        Err(FsError::NotFound)
    }

    /// Содержимое файла (для диагностики и тестов).
    pub fn read_data(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let path = normalize_path(path);
        let inner = self.inner.lock();
        inner
            .entries
            .iter()
            .find(|e| e.name == path && e.file_type == FileType::File)
            .map(|e| e.data.clone())
            .ok_or(FsError::NotFound)
    }
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn mount(&mut self) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner.mounted = true;
        // Создаём корневую директорию
        if !inner.entries.iter().any(|e| e.name == "/") {
            inner.entries.push(RamFsEntry::new_dir("/"));
        }
        Ok(())
    }

    fn unmount(&mut self) -> Result<(), FsError> {
        self.inner.lock().mounted = false;
        Ok(())
    }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let path = normalize_path(path);
        let inner = self.inner.lock();

        if inner
            .entries
            .iter()
            .any(|e| e.name == path && e.file_type == FileType::File)
        {
            drop(inner);
            return Ok(Box::new(RamFsFile {
                inner: self.inner.clone(),
                path,
                position: 0,
                writable: true,
            }));
        }

        Err(FsError::NotFound)
    }

    fn create(&mut self, path: &str) -> Result<Box<dyn File>, FsError> {
        let path = normalize_path(path);
        let mut inner = self.inner.lock();

        if inner.entries.iter().any(|e| e.name == path) {
            return Err(FsError::AlreadyExists);
        }
        if !parent_exists(&inner.entries, &path) {
            return Err(FsError::NotFound);
        }

        inner.entries.push(RamFsEntry::new_file(&path));
        drop(inner);

        Ok(Box::new(RamFsFile {
            inner: self.inner.clone(),
            path,
            position: 0,
            writable: true,
        }))
    }

    fn remove(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Err(FsError::PermissionDenied);
        }
        let mut inner = self.inner.lock();

        if let Some(pos) = inner.entries.iter().position(|e| e.name == path) {
            if inner.entries[pos].file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            inner.entries.remove(pos);
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }

    fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Err(FsError::AlreadyExists);
        }
        let mut inner = self.inner.lock();

        if inner.entries.iter().any(|e| e.name == path) {
            return Err(FsError::AlreadyExists);
        }
        if !parent_exists(&inner.entries, &path) {
            return Err(FsError::NotFound);
        }

        inner.entries.push(RamFsEntry::new_dir(&path));
        Ok(())
    }

    fn rmdir(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Err(FsError::PermissionDenied);
        }
        let mut inner = self.inner.lock();

        let Some(pos) = inner.entries.iter().position(|e| e.name == path) else {
            return Err(FsError::NotFound);
        };
        if inner.entries[pos].file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        // Проверяем, пуста ли директория. Сравнение по границе компонента:
        // иначе `/foo` считался бы родителем `/foobar`.
        let prefix = format!("{}/", path);
        let has_children = inner
            .entries
            .iter()
            .any(|e| e.name != path && e.name.starts_with(&prefix));

        if has_children {
            return Err(FsError::NotEmpty);
        }

        inner.entries.remove(pos);
        Ok(())
    }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let path = normalize_path(path);
        let inner = self.inner.lock();

        if !inner.entries.iter().any(|e| e.name == path) {
            return Err(FsError::NotFound);
        }

        let prefix = if path == "/" {
            String::from("/")
        } else {
            format!("{}/", path)
        };

        let mut entries = Vec::new();
        for entry in &inner.entries {
            if entry.name == path || !entry.name.starts_with(&prefix) {
                continue;
            }
            let relative = &entry.name[prefix.len()..];
            // Только прямые потомки.
            if relative.is_empty() || relative.contains('/') {
                continue;
            }

            let perms = if entry.file_type == FileType::Directory {
                0o755
            } else {
                0o644
            };
            entries.push(DirEntry {
                name: relative.to_string(),
                file_type: entry.file_type,
                size: entry.data.len(),
                permissions: Permissions::new(perms),
                uid: 0,
                gid: 0,
            });
        }

        Ok(entries)
    }

    fn exists(&self, path: &str) -> bool {
        let path = normalize_path(path);
        self.inner.lock().entries.iter().any(|e| e.name == path)
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let path = normalize_path(path);
        let inner = self.inner.lock();

        for entry in &inner.entries {
            if entry.name == path {
                let perms = if entry.file_type == FileType::Directory {
                    0o755
                } else {
                    0o644
                };
                return Ok(FileMetadata {
                    name: entry.name.clone(),
                    file_type: entry.file_type,
                    size: entry.data.len(),
                    permissions: Permissions::new(perms),
                    uid: 0,
                    gid: 0,
                    created: entry.created,
                    modified: entry.modified,
                });
            }
        }

        Err(FsError::NotFound)
    }
}

/// Существует ли родительский каталог для `path`.
fn parent_exists(entries: &[RamFsEntry], path: &str) -> bool {
    let parent = match path.rfind('/') {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => "/",
    };
    entries
        .iter()
        .any(|e| e.name == parent && e.file_type == FileType::Directory)
}

/// Файл в RamFS. Держит путь и общее состояние ФС, а не копию данных.
pub struct RamFsFile {
    inner: Arc<Mutex<RamFsInner>>,
    path: String,
    position: u64,
    writable: bool,
}

impl RamFsFile {
    fn entry_len(inner: &RamFsInner, path: &str) -> usize {
        inner
            .entries
            .iter()
            .find(|e| e.name == path && e.file_type == FileType::File)
            .map(|e| e.data.len())
            .unwrap_or(0)
    }
}

impl File for RamFsFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let entry = inner
            .entries
            .iter()
            .find(|e| e.name == self.path && e.file_type == FileType::File)
            .ok_or(FsError::NotFound)?;

        let pos = self.position as usize;
        if pos >= entry.data.len() {
            return Ok(0);
        }

        let to_read = buf.len().min(entry.data.len() - pos);
        buf[..to_read].copy_from_slice(&entry.data[pos..pos + to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        if !self.writable {
            return Err(FsError::ReadOnly);
        }

        let mut inner = self.inner.lock();
        let entry = inner
            .entries
            .iter_mut()
            .find(|e| e.name == self.path && e.file_type == FileType::File)
            .ok_or(FsError::NotFound)?;

        let pos = self.position as usize;
        if pos + buf.len() > entry.data.len() {
            entry.data.resize(pos + buf.len(), 0);
        }
        entry.data[pos..pos + buf.len()].copy_from_slice(buf);
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let len = Self::entry_len(&self.inner.lock(), &self.path);
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(offset) => self.position as i64 + offset,
            SeekFrom::End(offset) => len as i64 + offset,
        };

        if new_pos < 0 {
            return Err(FsError::InvalidPath);
        }

        self.position = new_pos as u64;
        Ok(self.position)
    }

    fn position(&self) -> u64 {
        self.position
    }

    fn size(&self) -> usize {
        Self::entry_len(&self.inner.lock(), &self.path)
    }
}

/// Нормализация пути: `/a/./b/../c` → `/a/c`, всегда абсолютный.
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        return String::from("/");
    }

    let mut normalized = String::new();
    for part in parts {
        normalized.push('/');
        normalized.push_str(part);
    }
    normalized
}
