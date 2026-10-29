pub mod vfs;
pub mod mount;
pub mod ramfs;
pub mod fat32;
pub mod ext2;
pub mod ntfs;
pub mod disk;
pub mod partition;
pub mod blockdev;
pub mod disk_manager;
pub mod procfs;
pub mod sysfs;
pub mod devtmpfs;

pub use vfs::*;
pub use ramfs::RamFs;
pub use fat32::Fat32Fs;
pub use ext2::Ext2Fs;
pub use ntfs::NtfsFs;
pub use disk::{DiskIo, RamDisk};
pub use disk_manager::DiskManager;

use spin::{Lazy, Mutex};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::format;

/// Единственный корневой RamFS. Раньше в VFS монтировался второй, пустой
/// экземпляр, из-за чего `ls /` был пуст, а `write_file` писал в копию `Vec`.
static ROOT_RAMFS: Lazy<Mutex<ramfs::RamFs>> = Lazy::new(|| {
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
    let mut ramfs = ROOT_RAMFS.lock();
    ramfs.mount().ok();

    // Канонический набор каталогов. Раньше здесь был «отладочный» набор
    // (/os, /os/kernel, /os/boot, /lib, /apps/*), который дублировал данные
    // команд version/uname/mount и никем не читался.
    let dirs = [
        "/dev", "/proc", "/sys", "/tmp", "/mnt", "/cfg",
        "/var", "/var/log",
        "/home", "/home/root", "/home/user",
    ];
    for dir in &dirs {
        ramfs.mkdir(dir).ok();
    }

    // /cfg — единственное место конфигурации (hostname читает его напрямую).
    let files: [(&str, &[u8]); 2] = [
        ("/cfg/hostname", b"vibra"),
        (
            "/cfg/passwd",
            b"root:x:0:0:root:/home/root:/bin/sh\nuser:x:1000:1000:user:/home/user:/bin/sh\n",
        ),
    ];

    for (path, data) in &files {
        let _ = ramfs.remove(path);
        if ramfs.create(path).is_ok() {
            let _ = ramfs.write_data(path, data);
        }
    }

    // Инициализируем виртуальные ФС и монтируем в VFS
    let vfs = &VFS_MANAGER;

    // / — корневой RamFS. Монтируется первым и является тем же экземпляром,
    // который наполняется выше: legacy-API и VFS видят одни и те же файлы.
    match vfs.mount("/", Box::new(ramfs.clone()), false) {
        Ok(()) => crate::println!("[FS] root ramfs mounted at /"),
        Err(e) => crate::println!("[FS] root ramfs mount failed: {:?}", e),
    }

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

    // /cfg/fstab собирается из реальной таблицы монтирования: статический
    // текст дублировал `mount` и мог разойтись с фактическим состоянием.
    write_fstab(&mut ramfs);

    // / — root ramfs
    // (уже смонтирован через LEGACY_RAMFS)

    // Принудительная инициализация Lazy
    Lazy::force(&VFS_MANAGER);
    Lazy::force(&DISK_MANAGER);

    // Блочные устройства: SATA-диски + таблицы разделов (MBR/GPT).
    // Вызывается после drivers::init(), поэтому AHCI уже проинициализирован.
    let found = {
        let mut mgr = DISK_MANAGER.lock();
        blockdev::probe(&mut mgr)
    };
    if found > 0 {
        crate::println!("[FS] block devices registered: {}", found);
    }
}

/// Пересобрать `/cfg/fstab` из текущей таблицы монтирования.
fn write_fstab(ramfs: &mut ramfs::RamFs) {
    let mut body = String::from("# mountpoint  filesystem\n");
    // Корень — RamFS, смонтированный напрямую (в таблице VFS его нет).
    body.push_str("/  ramfs  rw\n");
    for entry in list_mounts() {
        body.push_str(&entry);
        body.push('\n');
    }
    let _ = ramfs.remove("/cfg/fstab");
    if ramfs.create("/cfg/fstab").is_ok() {
        let _ = ramfs.write_data("/cfg/fstab", body.as_bytes());
    }
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

/// Предел на чтение файла целиком: `cat` большого файла не должен съесть кучу.
pub const MAX_READ_BYTES: usize = 32 * 1024 * 1024;

fn read_all(file: &mut dyn File) -> Result<Vec<u8>, FsError> {
    let size = file.size();
    if size > MAX_READ_BYTES {
        return Err(FsError::NotSupported);
    }
    let mut buf = alloc::vec![0u8; size];
    if size > 0 {
        file.read(&mut buf)?;
    }
    Ok(buf)
}

pub fn fs_count() -> usize {
    list_dir("/").len()
}

pub fn list_entries() -> Vec<DirEntry> {
    list_dir(&get_current_dir())
}

pub fn list_dir(path: &str) -> Vec<DirEntry> {
    // Сначала пробуем VFS (включая procfs/sysfs/devtmpfs).
    match vfs_readdir(path) {
        Ok(entries) => return entries,
        Err(FsError::NotFound) => {}
        Err(_) => return Vec::new(),
    }

    // Fallback: корневой RamFS, если путь не покрыт таблицей монтирования.
    let mut ramfs = ROOT_RAMFS.lock();
    if let Ok(entries) = ramfs.readdir(path) {
        entries
    } else {
        Vec::new()
    }
}

pub fn create_file(name: &str) -> Result<(), FsError> {
    let path = combine_path(&get_current_dir(), name);

    match vfs_create(&path) {
        Ok(_) => Ok(()),
        Err(FsError::NotFound) => {
            let mut ramfs = ROOT_RAMFS.lock();
            ramfs.create(&path)?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub fn read_file(name: &str) -> Result<Vec<u8>, FsError> {
    // Сначала пробуем VFS (включая procfs/sysfs/devtmpfs).
    match vfs_open(name) {
        Ok(mut file) => return read_all(file.as_mut()),
        Err(FsError::NotFound) => {}
        // Ошибку смонтированной ФС нельзя маскировать fallback-ом в ramfs.
        Err(e) => return Err(e),
    }

    // Fallback: корневой RamFS.
    let ramfs = ROOT_RAMFS.lock();
    let path = combine_path(&get_current_dir(), name);
    let mut file = ramfs.open(&path)?;
    read_all(file.as_mut())
}

pub fn write_file(name: &str, data: &[u8]) -> Result<(), FsError> {
    let path = combine_path(&get_current_dir(), name);

    // Перезаписываем: удаляем старый файл, если он есть. Ошибки чтения/удаления
    // на read-only томе должны дойти до пользователя, а не осесть в ramfs.
    match vfs_remove(&path) {
        Ok(()) | Err(FsError::NotFound) => {}
        Err(e) => return Err(e),
    }

    match vfs_create(&path) {
        Ok(mut file) => {
            file.write(data)?;
            Ok(())
        }
        Err(FsError::NotFound) => {
            let mut ramfs = ROOT_RAMFS.lock();
            let _ = ramfs.remove(&path);
            let mut file = ramfs.create(&path)?;
            file.write(data)?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub fn create_dir(name: &str) -> Result<(), FsError> {
    let path = combine_path(&get_current_dir(), name);

    // Сначала VFS: если путь попадает в смонтированный FAT32, каталог должен
    // появиться именно там. Раньше вызов всегда уходил в LEGACY_RAMFS, и
    // `mkdir /mnt/x` на диске не создавал ничего.
    match vfs_mkdir(&path) {
        Ok(()) => return Ok(()),
        // Путь не принадлежит ни одной точке монтирования — это нормально
        // для корневого RamFS.
        Err(FsError::NotFound) => {}
        // Том только для чтения или не умеет каталоги: пробовать RamFS
        // бессмысленно, ошибка должна дойти до пользователя.
        Err(e) => return Err(e),
    }

    let mut ramfs = ROOT_RAMFS.lock();
    ramfs.mkdir(&path)
}

/// Удалить каталог (только пустой).
///
/// Отдельно от `remove_entry`, потому что `rmdir` не должен сносить файлы:
/// иначе `rmdir file.txt` молча удалил бы файл.
pub fn remove_dir(name: &str) -> Result<(), FsError> {
    let path = combine_path(&get_current_dir(), name);

    match vfs_rmdir(&path) {
        Ok(()) => return Ok(()),
        Err(FsError::NotFound) => {}
        Err(e) => return Err(e),
    }

    let mut ramfs = ROOT_RAMFS.lock();
    ramfs.rmdir(&path)
}

pub fn remove_entry(name: &str) -> Result<(), FsError> {
    let path = combine_path(&get_current_dir(), name);

    // Файл или каталог на дисковом томе — через VFS.
    match vfs_remove(&path) {
        Ok(()) => return Ok(()),
        Err(FsError::NotFound) | Err(FsError::IsADirectory) => {}
        Err(e) => return Err(e),
    }
    match vfs_rmdir(&path) {
        Ok(()) => return Ok(()),
        Err(FsError::NotFound) | Err(FsError::NotADirectory) => {}
        Err(e) => return Err(e),
    }

    let mut ramfs = ROOT_RAMFS.lock();
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

    // Fallback: корневой RamFS.
    let ramfs = ROOT_RAMFS.lock();
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

/// Точка монтирования, в которую попадает `path` (самый длинный префикс).
///
/// `None` означает, что путь обслуживает не VFS, а legacy ramfs на `/`.
/// Нужна вызывающему коду, чтобы отличить «здесь вообще нет ФС» от
/// «ФС есть, но операция запрещена» — глотать второе нельзя.
pub fn mount_point_of(path: &str) -> Option<String> {
    let mt = VFS_MANAGER.mount_table.lock();
    mt.find_fs(path).map(|(idx, _)| mt.get(idx).path.clone())
}

/// Монтировать FAT32 диск (только чтение — запись на носитель пока запрещена)
pub fn mount_fat32(path: &str, disk: Box<dyn DiskIo>) -> Result<(), FsError> {
    let mut fatfs = Fat32Fs::new(disk)?;
    fatfs.mount()?;
    mount_fs(path, Box::new(fatfs), true)
}

/// Монтировать FAT32 диск с записью (используется log-приёмником и тестами).
pub fn mount_fat32_rw(path: &str, disk: Box<dyn DiskIo>) -> Result<(), FsError> {
    let mut fatfs = Fat32Fs::new_rw(disk)?;
    fatfs.mount()?;
    mount_fs(path, Box::new(fatfs), false)
}

/// Монтировать EXT2/3/4 диск (опционально readonly)
pub fn mount_ext(path: &str, disk: Box<dyn DiskIo>, readonly: bool) -> Result<(), FsError> {
    let mut extfs = Ext2Fs::new(disk)?;
    extfs.mount()?;
    mount_fs(path, Box::new(extfs), readonly)
}

/// Монтировать NTFS раздел (только чтение)
pub fn mount_ntfs(path: &str, disk: Box<dyn DiskIo>) -> Result<(), FsError> {
    let mut ntfs = NtfsFs::new(disk)?;
    ntfs.mount()?;
    mount_fs(path, Box::new(ntfs), true)
}

/// Смонтировать раздел/диск из реестра, определив ФС автоматически.
///
/// Возвращает имя смонтированной ФС (`"fat32"`, `"ext2"`, `"ntfs"`).
/// По умолчанию — только чтение; для FAT32 запись включается
/// `mount_auto_rw`.
pub fn mount_auto(path: &str, disk_id: usize) -> Result<&'static str, FsError> {
    mount_auto_mode(path, disk_id, false)
}

/// Автомонтирование FAT32 с записью (явный `rw` в команде mount).
pub fn mount_auto_rw(path: &str, disk_id: usize) -> Result<&'static str, FsError> {
    mount_auto_mode(path, disk_id, true)
}

fn mount_auto_mode(path: &str, disk_id: usize, write: bool) -> Result<&'static str, FsError> {
    // Тип ФС уже определён при probe, но если запись добавлена вручную
    // (`test-disk ram`), определяем на месте.
    let fs_type = {
        let mgr = get_disk_manager().lock();
        match mgr.info(disk_id) {
            Some((_, _, _, t)) => t,
            None => return Err(FsError::NotFound),
        }
    };

    let fs_type = if fs_type == "unknown" {
        let mut probe_disk = get_disk_manager().lock().open(disk_id).ok_or(FsError::NotFound)?;
        partition::detect_fs_type(probe_disk.as_mut())
    } else {
        fs_type
    };

    let disk = get_disk_manager().lock().open(disk_id).ok_or(FsError::NotFound)?;

    match fs_type {
        "fat32" => {
            // Запись на физический носитель включается только явным `rw`.
            let mut fs = if write {
                Fat32Fs::new_rw(disk)?
            } else {
                Fat32Fs::new(disk)?
            };
            fs.mount()?;
            mount_fs(path, Box::new(fs), !write)?;
            Ok("fat32")
        }
        "ext2" => {
            if write {
                return Err(FsError::ReadOnly);
            }
            let mut fs = Ext2Fs::new(disk)?;
            fs.mount()?;
            mount_fs(path, Box::new(fs), true)?;
            Ok("ext2")
        }
        "ntfs" => {
            if write {
                return Err(FsError::ReadOnly);
            }
            let mut fs = NtfsFs::new(disk)?;
            fs.mount()?;
            mount_fs(path, Box::new(fs), true)?;
            Ok("ntfs")
        }
        "exfat" => Err(FsError::NotSupported),
        _ => Err(FsError::NotSupported),
    }
}

/// Список смонтированных ФС
pub fn list_mounts() -> Vec<String> {
    VFS_MANAGER.mount_table.lock().list()
}

/// Проверить, смонтирована ли ФС
pub fn is_mounted(path: &str) -> bool {
    VFS_MANAGER.mount_table.lock().is_mounted(path)
}

/// Попытаться найти и открыть файл через VFS.
pub fn vfs_open(path: &str) -> Result<Box<dyn File>, FsError> {
    let mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        // Если относительный путь пустой, это корень
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        let mount = mt.get(idx);
        let file = mount.fs.open(target_path)?;
        if mount.readonly {
            Ok(Box::new(vfs::ReadOnlyFile { inner: file }))
        } else {
            Ok(file)
        }
    } else {
        Err(FsError::NotFound)
    }
}

/// Попытаться создать файл через VFS
pub fn vfs_create(path: &str) -> Result<Box<dyn File>, FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        if mt.get(idx).readonly {
            return Err(FsError::ReadOnly);
        }
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

/// Попытаться создать каталог через VFS.
///
/// Нужен, чтобы `mkdir /mnt/x` работал на смонтированном FAT32, а не только
/// на корневом RamFS: раньше `fs::create_dir` был жёстко привязан к
/// `LEGACY_RAMFS` и на дисковых томах каталог просто не появлялся.
pub fn vfs_mkdir(path: &str) -> Result<(), FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        if mt.get(idx).readonly {
            return Err(FsError::ReadOnly);
        }
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get_mut(idx).fs.mkdir(target_path)
    } else {
        Err(FsError::NotFound)
    }
}

/// Удалить каталог через VFS.
pub fn vfs_rmdir(path: &str) -> Result<(), FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        if mt.get(idx).readonly {
            return Err(FsError::ReadOnly);
        }
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get_mut(idx).fs.rmdir(target_path)
    } else {
        Err(FsError::NotFound)
    }
}

/// Удалить файл через VFS.
pub fn vfs_remove(path: &str) -> Result<(), FsError> {
    let mut mt = VFS_MANAGER.mount_table.lock();

    if let Some((idx, relative_path)) = mt.find_fs(path) {
        if mt.get(idx).readonly {
            return Err(FsError::ReadOnly);
        }
        let target_path = if relative_path.is_empty() || relative_path == "/" {
            "/"
        } else {
            &relative_path
        };
        mt.get_mut(idx).fs.remove(target_path)
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
