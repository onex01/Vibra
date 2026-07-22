pub mod vfs;
pub mod mount;
pub mod ramfs;
pub mod fat32;
pub mod ext2;
pub mod disk;
pub mod disk_manager;
pub mod procfs;
pub mod sysfs;
pub mod devtmpfs;

pub use vfs::*;
pub use ramfs::RamFs;
pub use fat32::Fat32Fs;
pub use ext2::Ext2Fs;
pub use disk::{DiskIo, RamDisk};
pub use disk_manager::DiskManager;

use spin::{Lazy, Mutex};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::format;

static LEGACY_RAMFS: Lazy<Mutex<ramfs::RamFs>> = Lazy::new(|| {
    Mutex::new(ramfs::RamFs::new())
});

static CURRENT_DIR: Lazy<Mutex<Option<String>>> = Lazy::new(|| {
    Mutex::new(Some(String::from("/")))
});

// Глобальный VFS-менеджер (Lazy инициализируется при первом обращении)
static VFS_MANAGER: Lazy<VfsManager> = Lazy::new(VfsManager::new);

// Глобальный менеджер дисков
static DISK_MANAGER: Lazy<Mutex<DiskManager>> = Lazy::new(|| {
    Mutex::new(DiskManager::new())
});

/// Инициализация файловой системы (вызывается из main.rs)
pub fn init_filesystem() {
    let mut ramfs = LEGACY_RAMFS.lock();
    ramfs.mount().ok();

    // Уникальная структура каталогов Vibra OS
    let dirs = [
        "/os", "/os/kernel", "/os/boot", "/os/services",
        "/dev", "/cfg", "/lib",
        "/apps", "/apps/bin", "/apps/data",
        "/home", "/home/root",
        "/var", "/var/log", "/var/cache",
        "/tmp", "/mnt",
        "/proc", "/sys", "/sys/kernel",
    ];
    for dir in &dirs {
        ramfs.mkdir(dir).ok();
    }

    // Системные файлы
    let files: [(&str, &[u8]); 7] = [
        ("/cfg/hostname", b"vibra"),
        ("/cfg/passwd", b"root:x:0:0:root:/home/root:/bin/sh\n"),
        ("/cfg/fstab", b"# device  mount  type  options  dump  pass\n/         ramfs  rw    0        0     0\n/dev      devtmpfs rw 0        0     0\n/proc     procfs  ro   0        0     0\n/sys      sysfs   ro   0        0     0\n"),
        ("/os/kernel/version", b"0.6.0\n"),
        ("/os/kernel/name", b"Vibra\n"),
        ("/os/boot/loader", b"Limine (UEFI)\n"),
        ("/os/boot/config", b"timeout: 0\ngraphics: yes\n"),
    ];

    for (path, data) in &files {
        let _ = ramfs.remove(path);
        if let Ok(_) = ramfs.create(path) {
            let _ = ramfs.write_data(path, data);
        }
    }

    // Инициализируем виртуальные ФС и монтируем в VFS
    let vfs = &VFS_MANAGER;

    // /proc — procfs
    let procfs = procfs::ProcFs::new();
    match vfs.mount("/proc", Box::new(procfs), true) {
        Ok(()) => crate::println!("[FS] procfs mounted at /proc"),
        Err(e) => crate::println!("[FS] procfs mount failed: {:?}", e),
    }

    // /sys — sysfs
    let sysfs = sysfs::SysFs::new();
    match vfs.mount("/sys", Box::new(sysfs), true) {
        Ok(()) => crate::println!("[FS] sysfs mounted at /sys"),
        Err(e) => crate::println!("[FS] sysfs mount failed: {:?}", e),
    }

    // /dev — devtmpfs
    let devtmpfs = devtmpfs::DevTmpFs::new();
    match vfs.mount("/dev", Box::new(devtmpfs), false) {
        Ok(()) => crate::println!("[FS] devtmpfs mounted at /dev"),
        Err(e) => crate::println!("[FS] devtmpfs mount failed: {:?}", e),
    }

    // / — root ramfs
    // (уже смонтирован через LEGACY_RAMFS)

    // Принудительная инициализация Lazy
    Lazy::force(&VFS_MANAGER);
    Lazy::force(&DISK_MANAGER);
}

// ==========================================
// LEGACY API (Для совместимости со старыми командами shell)
// ==========================================

pub fn get_current_dir() -> String {
    CURRENT_DIR.lock().clone().unwrap_or_else(|| String::from("/"))
}

pub fn set_current_dir(path: &str) {
    *CURRENT_DIR.lock() = Some(String::from(path));
}

pub fn fs_count() -> usize {
    let mut ramfs = LEGACY_RAMFS.lock();
    if let Ok(entries) = ramfs.readdir("/") {
        entries.len()
    } else {
        0
    }
}

pub fn list_entries() -> Vec<DirEntry> {
    let mut ramfs = LEGACY_RAMFS.lock();
    let dir = get_current_dir();
    if let Ok(entries) = ramfs.readdir(&dir) {
        entries
    } else {
        Vec::new()
    }
}

pub fn list_dir(path: &str) -> Vec<DirEntry> {
    // Сначала пробуем VFS
    if let Ok(entries) = vfs_readdir(path) {
        return entries;
    }

    // Fallback: legacy RamFS
    let mut ramfs = LEGACY_RAMFS.lock();
    if let Ok(entries) = ramfs.readdir(path) {
        entries
    } else {
        Vec::new()
    }
}

pub fn create_file(name: &str) -> Result<(), FsError> {
    let mut ramfs = LEGACY_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    ramfs.create(&path)?;
    Ok(())
}

pub fn read_file(name: &str) -> Result<Vec<u8>, FsError> {
    // Сначала пробуем VFS (включая procfs/sysfs/devtmpfs)
    if let Ok(mut file) = vfs_open(name) {
        let size = file.size();
        let mut buf = alloc::vec![0u8; size];
        if size > 0 {
            file.read(&mut buf)?;
        }
        return Ok(buf);
    }

    // Fallback: legacy RamFS
    let ramfs = LEGACY_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    let mut file = ramfs.open(&path)?;
    let size = file.size();
    let mut buf = alloc::vec![0u8; size];
    if size > 0 {
        file.read(&mut buf)?;
    }
    Ok(buf)
}

pub fn write_file(name: &str, data: &[u8]) -> Result<(), FsError> {
    let mut ramfs = LEGACY_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    let _ = ramfs.remove(&path); // Перезаписываем
    let mut file = ramfs.create(&path)?;
    file.write(data)?;
    Ok(())
}

pub fn create_dir(name: &str) -> Result<(), FsError> {
    let mut ramfs = LEGACY_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    ramfs.mkdir(&path)
}

pub fn remove_entry(name: &str) -> Result<(), FsError> {
    let mut ramfs = LEGACY_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    // Пробуем удалить как файл, если не вышло - как директорию
    if ramfs.remove(&path).is_err() {
        ramfs.rmdir(&path)?;
    }
    Ok(())
}

pub fn dir_exists(path: &str) -> bool {
    // Сначала пробуем VFS
    if vfs_exists(path) {
        return true;
    }

    // Fallback: legacy RamFS
    let ramfs = LEGACY_RAMFS.lock();
    let full_path = combine_path(&get_current_dir(), path);
    if let Ok(meta) = ramfs.stat(&full_path) {
        meta.file_type == FileType::Directory
    } else {
        false
    }
}

fn combine_path(base: &str, name: &str) -> String {
    if name.starts_with('/') {
        String::from(name)
    } else if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base, name)
    }
}

// ==========================================
// NEW API - Виртуальная файловая система
// ==========================================

/// Получить VFS менеджер
pub fn get_vfs_manager() -> &'static VfsManager {
    &VFS_MANAGER
}

/// Получить Disk менеджер
pub fn get_disk_manager() -> &'static Mutex<DiskManager> {
    &DISK_MANAGER
}

/// Монтировать ФС по пути
pub fn mount_fs(path: &str, fs: Box<dyn FileSystem>, readonly: bool) -> Result<(), FsError> {
    VFS_MANAGER.mount(path, fs, readonly)
}

/// Монтировать FAT32 диск
pub fn mount_fat32(path: &str, disk: Box<dyn DiskIo>) -> Result<(), FsError> {
    let mut fatfs = Fat32Fs::new(disk)?;
    fatfs.mount()?;
    mount_fs(path, Box::new(fatfs), false)
}

/// Монтировать EXT2/3/4 диск (опционально readonly)
pub fn mount_ext(path: &str, disk: Box<dyn DiskIo>, readonly: bool) -> Result<(), FsError> {
    let mut extfs = Ext2Fs::new(disk)?;
    extfs.mount()?;
    mount_fs(path, Box::new(extfs), readonly)
}

/// Список смонтированных ФС
pub fn list_mounts() -> Vec<String> {
    VFS_MANAGER.mount_table.lock().list()
}

/// Проверить, смонтирована ли ФС
pub fn is_mounted(path: &str) -> bool {
    VFS_MANAGER.mount_table.lock().is_mounted(path)
}

/// Попытаться найти и открыть файл через VFS
pub fn vfs_open(path: &str) -> Result<Box<dyn File>, FsError> {
    let mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        // Если относительный путь пустой, это корень
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get(idx).fs.open(target_path)
    } else {
        Err(FsError::NotFound)
    }
}

/// Попытаться создать файл через VFS
pub fn vfs_create(path: &str) -> Result<Box<dyn File>, FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get_mut(idx).fs.create(target_path)
    } else {
        Err(FsError::NotFound)
    }
}

/// Получить содержимое директории через VFS
pub fn vfs_readdir(path: &str) -> Result<Vec<DirEntry>, FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get_mut(idx).fs.readdir(target_path)
    } else {
        Err(FsError::NotFound)
    }
}

/// Проверить существование файла через VFS
pub fn vfs_exists(path: &str) -> bool {
    if let Ok(meta) = vfs_stat(path) {
        !meta.name.is_empty()
    } else {
        false
    }
}

/// Получить метаданные файла через VFS
pub fn vfs_stat(path: &str) -> Result<FileMetadata, FsError> {
    let mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get(idx).fs.stat(target_path)
    } else {
        Err(FsError::NotFound)
    }
}
