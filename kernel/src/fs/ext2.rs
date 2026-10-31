// EXT2/EXT3/EXT4 File System Driver — чтение.
//
// Исторически файл назывался ext2.rs и умел только классические block
// pointers ext2. Современный ext4 по умолчанию использует extents, 64-битные
// group descriptors и metadata checksums, поэтому `mount` рапортовал успех, а
// каталоги читались пустыми (i_block[0..12] — это заголовок extent-дерева, а
// не номера блоков).
//
// Что поддерживается:
//   * ext2 (классические indirect-блоки, rev0/rev1);
//   * ext3 (то же плюс journal — для чтения он не нужен);
//   * ext4: extents, 64bit, flex_bg, dir_index (линейный обход), metadata_csum,
//     huge_file, extra_isize, large_dir.
//
// Что сознательно не поддерживается (возвращается NotSupported, а не «пустой
// каталог»): inline_data, encrypt, bigalloc, meta_bg, compression, journal_dev.
// Запись не поддерживается вовсе: все мутирующие операции возвращают ReadOnly.

use super::vfs::*;
use super::disk::DiskIo;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::boxed::Box;
use alloc::vec;

const EXT_MAGIC: u16 = 0xEF53;
const SECTOR_SIZE: usize = 512;

/// Предел на файл, который драйвер готов держать в памяти целиком.
/// `cat` большого файла не должен съесть всю кучу.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

// --- Feature flags (superblock offsets 0x5C/0x60/0x64) ---
const COMPAT_HAS_JOURNAL: u32 = 0x0004;
const COMPAT_DIR_INDEX: u32 = 0x0020;

const INCOMPAT_COMPRESSION: u32 = 0x0001;
const INCOMPAT_FILETYPE: u32 = 0x0002;
const INCOMPAT_RECOVER: u32 = 0x0004;
const INCOMPAT_JOURNAL_DEV: u32 = 0x0008;
const INCOMPAT_META_BG: u32 = 0x0010;
const INCOMPAT_EXTENTS: u32 = 0x0040;
const INCOMPAT_64BIT: u32 = 0x0080;
const INCOMPAT_MMP: u32 = 0x0100;
const INCOMPAT_FLEX_BG: u32 = 0x0200;
const INCOMPAT_EA_INODE: u32 = 0x0400;
const INCOMPAT_DIRDATA: u32 = 0x1000;
const INCOMPAT_CSUM_SEED: u32 = 0x2000;
const INCOMPAT_LARGEDIR: u32 = 0x4000;
const INCOMPAT_INLINE_DATA: u32 = 0x8000;
const INCOMPAT_ENCRYPT: u32 = 0x10000;
const INCOMPAT_CASEFOLD: u32 = 0x20000;

const RO_COMPAT_BIGALLOC: u32 = 0x0200;

// --- Inode flags ---
const INODE_EXTENTS: u32 = 0x0008_0000;
const INODE_INLINE_DATA: u32 = 0x1000_0000;

// --- Extent tree ---
const EXTENT_MAGIC: u16 = 0xF30A;
const EXTENT_HEADER_LEN: usize = 12;
const EXTENT_ENTRY_LEN: usize = 12;
/// Бит в `ee_len`: длина больше этого значения означает «unwritten».
const EXTENT_INIT_MAX_LEN: u16 = 0x8000;
const MAX_EXTENT_DEPTH: u32 = 8;

// --- File types ---
const S_IFMT: u16 = 0xF000;
const S_IFDIR: u16 = 0x4000;
const S_IFREG: u16 = 0x8000;
const S_IFLNK: u16 = 0xA000;
const S_IFCHR: u16 = 0x2000;
const S_IFBLK: u16 = 0x6000;
const S_IFIFO: u16 = 0x1000;
const S_IFSOCK: u16 = 0xC000;

pub struct ExtSuperblock {
    pub inodes_count: u32,
    pub blocks_count: u64,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub rev_level: u32,
    pub inode_size: u16,
    pub magic: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub desc_size: u16,
    pub first_ino: u32,
}

fn u16_at(buf: &[u8], off: usize) -> u16 {
    match buf.get(off..off + 2) {
        Some(s) => u16::from_le_bytes([s[0], s[1]]),
        None => 0,
    }
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    match buf.get(off..off + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

impl ExtSuperblock {
    /// Разбор 1024-байтного суперблока (смещения — от его начала).
    fn parse(data: &[u8]) -> Result<Self, FsError> {
        if data.len() < 1024 {
            return Err(FsError::IoError);
        }
        let magic = u16_at(data, 56);
        if magic != EXT_MAGIC {
            return Err(FsError::InvalidPath);
        }

        let rev_level = u32_at(data, 76);
        let feature_incompat = u32_at(data, 96);
        let feature_ro_compat = u32_at(data, 100);

        // Верхняя половина счётчика блоков есть только в 64bit-ФС.
        let blocks_hi = if feature_incompat & INCOMPAT_64BIT != 0 {
            u32_at(data, 0x150) as u64
        } else {
            0
        };

        let mut inode_size = u16_at(data, 88);
        if rev_level == 0 || inode_size < 128 {
            inode_size = 128;
        }

        let mut desc_size = 32u16;
        if feature_incompat & INCOMPAT_64BIT != 0 {
            let raw = u16_at(data, 0xFE);
            // ext4 допускает 64..block_size; 0 в старых образах означает 64.
            desc_size = if raw == 0 { 64 } else { raw };
        }

        Ok(Self {
            inodes_count: u32_at(data, 0),
            blocks_count: (u32_at(data, 4) as u64) | (blocks_hi << 32),
            free_blocks_count: u32_at(data, 12),
            free_inodes_count: u32_at(data, 16),
            first_data_block: u32_at(data, 20),
            log_block_size: u32_at(data, 24),
            blocks_per_group: u32_at(data, 32),
            inodes_per_group: u32_at(data, 40),
            rev_level,
            inode_size,
            magic,
            feature_compat: u32_at(data, 92),
            feature_incompat,
            feature_ro_compat,
            desc_size,
            first_ino: u32_at(data, 84),
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

    /// Человеческое имя ФС по набору фич.
    fn fs_name(&self) -> &'static str {
        if self.feature_incompat & (INCOMPAT_EXTENTS | INCOMPAT_64BIT) != 0
            || self.feature_ro_compat & 0x0200 != 0 // METADATA_CSUM
            || self.feature_incompat & INCOMPAT_FLEX_BG != 0
        {
            "ext4"
        } else if self.feature_compat & COMPAT_HAS_JOURNAL != 0 {
            "ext3"
        } else {
            "ext2"
        }
    }

    /// Фичи, без которых драйвер не сможет корректно прочитать данные.
    fn check_supported(&self) -> Result<(), FsError> {
        let unsupported = INCOMPAT_COMPRESSION
            | INCOMPAT_JOURNAL_DEV
            | INCOMPAT_META_BG
            | INCOMPAT_INLINE_DATA
            | INCOMPAT_ENCRYPT;
        if self.feature_incompat & unsupported != 0 {
            return Err(FsError::NotSupported);
        }
        if self.feature_ro_compat & RO_COMPAT_BIGALLOC != 0 {
            return Err(FsError::NotSupported);
        }
        Ok(())
    }
}

pub struct Ext2Fs {
    disk: spin::Mutex<Box<dyn DiskIo>>,
    superblock: ExtSuperblock,
    block_size: u32,
    /// Размер group descriptor: 32 для ext2, 64 при 64bit.
    desc_size: usize,
    /// inodes_per_block, посчитанный один раз.
    inodes_per_block: usize,
    fs_name: &'static str,
    mounted: bool,
}

impl Ext2Fs {
    pub fn new(mut disk: Box<dyn DiskIo>) -> Result<Self, FsError> {
        // Суперблок ext2/3/4 всегда лежит по смещению 1024 байта (сектор 2 при
        // 512-байтных секторах). Раньше читался сектор 1, из-за чего магия
        // 0xEF53 искалась на 512 байт раньше и монтирование всегда падало.
        let mut buf = [0u8; 1024];
        disk.read(2, &mut buf).map_err(|_| FsError::IoError)?;
        let superblock = ExtSuperblock::parse(&buf)?;
        superblock.check_supported()?;

        // log_block_size — степень двойки: 0..=6 (1 КиБ..64 КиБ). Огромное
        // значение из повреждённого суперблока дало бы переполнение сдвига.
        if superblock.log_block_size > 6 {
            return Err(FsError::InvalidPath);
        }
        if superblock.blocks_per_group == 0 || superblock.inodes_per_group == 0 {
            return Err(FsError::InvalidPath);
        }
        let block_size = 1024u32 << superblock.log_block_size;

        let desc_size = superblock.desc_size as usize;
        if desc_size < 32 || desc_size > block_size as usize {
            return Err(FsError::InvalidPath);
        }

        let inode_size = superblock.inode_size_bytes();
        let inodes_per_block = (block_size as usize / inode_size).max(1);
        let fs_name = superblock.fs_name();

        let fs = Self {
            disk: spin::Mutex::new(disk),
            superblock,
            block_size,
            desc_size,
            inodes_per_block,
            fs_name,
            mounted: false,
        };

        // Проверяем, что корневой inode действительно читается. Иначе
        // `mount` снова рапортовал бы успех при несовместимом образе.
        let root = fs.read_inode_data(2)?;
        let mode = u16_at(&root, 0);
        if mode & S_IFMT != S_IFDIR {
            return Err(FsError::InvalidPath);
        }

        Ok(fs)
    }

    pub fn fs_type(&self) -> &'static str {
        self.fs_name
    }

    /// Публичная сводка для `diag`/`mount`: (inodes, blocks, free_blocks, name).
    pub fn summary(&self) -> (u32, u64, u32, &'static str) {
        (
            self.superblock.inodes_count,
            self.superblock.blocks_count,
            self.superblock.free_blocks_count,
            self.fs_name,
        )
    }

    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), FsError> {
        if block > (u64::MAX / self.block_size as u64) {
            return Err(FsError::IoError);
        }
        let sector = block * (self.block_size as u64 / SECTOR_SIZE as u64);
        self.disk.lock().read(sector, buf).map_err(|_| FsError::IoError)
    }

    /// Блок таблицы inode для группы `group`.
    ///
    /// Адрес берётся из Group Descriptor (bg_inode_table, смещение 8 в
    /// дескрипторе; при 64bit старшая половина — по смещению 0x28).
    fn inode_table_block(&self, group: u32) -> Result<u64, FsError> {
        let gdt_block = self.superblock.first_data_block as u64 + 1;
        let descs_per_block = (self.block_size as usize / self.desc_size).max(1);
        let block_for_group = gdt_block + (group as usize / descs_per_block) as u64;
        let index_in_block = (group as usize % descs_per_block) * self.desc_size;

        let mut buf = vec![0u8; self.block_size as usize];
        self.read_block(block_for_group, &mut buf)?;

        let mut table = u32_at(&buf, index_in_block + 8) as u64;
        if self.desc_size >= 64 {
            table |= (u32_at(&buf, index_in_block + 0x28) as u64) << 32;
        }
        if table == 0 {
            return Err(FsError::IoError);
        }
        Ok(table)
    }

    fn read_inode_data(&self, inode_num: u32) -> Result<Vec<u8>, FsError> {
        if inode_num == 0 || inode_num > self.superblock.inodes_count {
            return Err(FsError::NotFound);
        }
        let per_group = self.superblock.inodes_per_group.max(1);
        let group = (inode_num - 1) / per_group;
        let index = (inode_num - 1) % per_group;

        let inode_size = self.superblock.inode_size_bytes();
        let inodes_per_block = self.inodes_per_block.max(1);

        let table = self.inode_table_block(group)?;
        let block = table + (index as usize / inodes_per_block) as u64;
        let offset = (index as usize % inodes_per_block) * inode_size;

        let mut buf = vec![0u8; self.block_size as usize];
        self.read_block(block, &mut buf)?;
        if offset + inode_size > buf.len() {
            return Err(FsError::IoError);
        }
        Ok(buf[offset..offset + inode_size].to_vec())
    }

    fn inode_size_bytes(inode: &[u8]) -> u64 {
        let lo = u32_at(inode, 4) as u64;
        let hi = u32_at(inode, 108) as u64;
        lo | (hi << 32)
    }

    fn inode_mode(inode: &[u8]) -> u16 {
        u16_at(inode, 0)
    }

    fn inode_flags(inode: &[u8]) -> u32 {
        u32_at(inode, 32)
    }

    /// Все блоки файла/каталога по порядку.
    ///
    /// Нулевой элемент означает «дыру» (sparse-файл): такой блок не читается,
    /// но позиция в выходном буфере под него сохраняется.
    fn inode_blocks(&self, inode: &[u8], size: u64) -> Result<Vec<u64>, FsError> {
        let bs = self.block_size as u64;
        let needed = ((size + bs - 1) / bs) as usize;
        if needed == 0 {
            return Ok(Vec::new());
        }
        // Защита от повреждённого размера: больше 64 ГиБ на файл не читаем.
        if needed > (MAX_FILE_BYTES as usize / self.block_size as usize) + 1 {
            return Err(FsError::NotSupported);
        }

        let flags = Self::inode_flags(inode);
        if flags & INODE_INLINE_DATA != 0 {
            return Err(FsError::NotSupported);
        }
        if flags & INODE_EXTENTS != 0 {
            self.extent_blocks(inode, needed)
        } else {
            self.indirect_blocks(inode, needed)
        }
    }

    /// Классические ext2 block pointers: 12 прямых + 1/2/3 косвенных.
    fn indirect_blocks(&self, inode: &[u8], needed: usize) -> Result<Vec<u64>, FsError> {
        let per_block = (self.block_size as usize / 4).max(1);
        let mut out: Vec<u64> = Vec::with_capacity(needed);
        let mut buf = vec![0u8; self.block_size as usize];

        let ptr = |i: usize| -> u64 { u32_at(inode, 40 + i * 4) as u64 };

        for i in 0..12 {
            if out.len() >= needed {
                return Ok(out);
            }
            out.push(ptr(i));
        }

        // Одиночная косвенная (i_block[12]).
        if out.len() < needed {
            let indirect = ptr(12);
            if indirect == 0 {
                while out.len() < needed && out.len() < 12 + per_block {
                    out.push(0);
                }
            } else {
                self.read_block(indirect, &mut buf)?;
                for i in 0..per_block {
                    if out.len() >= needed {
                        break;
                    }
                    out.push(u32_at(&buf, i * 4) as u64);
                }
            }
        }

        // Двойная косвенная (i_block[13]).
        if out.len() < needed {
            let dindirect = ptr(13);
            if dindirect == 0 {
                let limit = 12 + per_block + per_block * per_block;
                while out.len() < needed && out.len() < limit {
                    out.push(0);
                }
            } else {
                let mut l1 = vec![0u8; self.block_size as usize];
                self.read_block(dindirect, &mut l1)?;
                for i in 0..per_block {
                    if out.len() >= needed {
                        break;
                    }
                    let second = u32_at(&l1, i * 4) as u64;
                    if second == 0 {
                        for _ in 0..per_block {
                            if out.len() >= needed {
                                break;
                            }
                            out.push(0);
                        }
                        continue;
                    }
                    self.read_block(second, &mut buf)?;
                    for j in 0..per_block {
                        if out.len() >= needed {
                            break;
                        }
                        out.push(u32_at(&buf, j * 4) as u64);
                    }
                }
            }
        }

        // Тройная косвенная (i_block[14]) — нужна только очень большим файлам.
        if out.len() < needed {
            let tindirect = ptr(14);
            if tindirect == 0 {
                let limit = 12 + per_block + per_block * per_block
                    + per_block * per_block * per_block;
                while out.len() < needed && out.len() < limit {
                    out.push(0);
                }
            } else {
                let mut l1 = vec![0u8; self.block_size as usize];
                let mut l2 = vec![0u8; self.block_size as usize];
                self.read_block(tindirect, &mut l1)?;
                'outer: for i in 0..per_block {
                    let second = u32_at(&l1, i * 4) as u64;
                    if second == 0 {
                        for _ in 0..(per_block * per_block) {
                            if out.len() >= needed {
                                break 'outer;
                            }
                            out.push(0);
                        }
                        continue;
                    }
                    self.read_block(second, &mut l2)?;
                    for j in 0..per_block {
                        let third = u32_at(&l2, j * 4) as u64;
                        if third == 0 {
                            for _ in 0..per_block {
                                if out.len() >= needed {
                                    break 'outer;
                                }
                                out.push(0);
                            }
                            continue;
                        }
                        self.read_block(third, &mut buf)?;
                        for k in 0..per_block {
                            if out.len() >= needed {
                                break 'outer;
                            }
                            out.push(u32_at(&buf, k * 4) as u64);
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    /// Extent-дерево ext4: рекурсивно собирает листовые экстенты.
    fn extent_blocks(&self, inode: &[u8], needed: usize) -> Result<Vec<u64>, FsError> {
        let mut extents: Vec<Extent> = Vec::new();
        // Заголовок дерева лежит прямо в i_block (60 байт).
        let header = &inode[40..100.min(inode.len())];
        self.walk_extent_node(header, &mut extents, 0)?;

        let mut out = vec![0u64; needed];
        for ext in &extents {
            if ext.unwritten {
                continue;
            }
            for i in 0..ext.len as usize {
                let logical = ext.logical as usize + i;
                if logical >= needed {
                    break;
                }
                let physical = match ext.physical.checked_add(i as u64) {
                    Some(value) => value,
                    None => return Err(FsError::IoError),
                };
                out[logical] = physical;
            }
        }
        Ok(out)
    }

    fn walk_extent_node(
        &self,
        node: &[u8],
        out: &mut Vec<Extent>,
        depth: u32,
    ) -> Result<(), FsError> {
        if depth > MAX_EXTENT_DEPTH {
            return Err(FsError::NotSupported);
        }
        if node.len() < EXTENT_HEADER_LEN {
            return Err(FsError::IoError);
        }
        if u16_at(node, 0) != EXTENT_MAGIC {
            return Err(FsError::InvalidPath);
        }
        let entries = u16_at(node, 2) as usize;
        let max = u16_at(node, 4) as usize;
        let node_depth = u16_at(node, 6);
        if entries > max {
            return Err(FsError::InvalidPath);
        }
        if EXTENT_HEADER_LEN + entries * EXTENT_ENTRY_LEN > node.len() {
            return Err(FsError::InvalidPath);
        }

        for i in 0..entries {
            let off = EXTENT_HEADER_LEN + i * EXTENT_ENTRY_LEN;
            if node_depth == 0 {
                let logical = u32_at(node, off);
                let raw_len = u16_at(node, off + 4);
                let start_hi = u16_at(node, off + 6) as u64;
                let start_lo = u32_at(node, off + 8) as u64;
                let unwritten = raw_len > EXTENT_INIT_MAX_LEN;
                let len = if unwritten {
                    raw_len - EXTENT_INIT_MAX_LEN
                } else {
                    raw_len
                };
                if len == 0 {
                    continue;
                }
                out.push(Extent {
                    logical,
                    physical: start_lo | (start_hi << 32),
                    len,
                    unwritten,
                });
            } else {
                let leaf_lo = u32_at(node, off + 4) as u64;
                let leaf_hi = u16_at(node, off + 8) as u64;
                let leaf = leaf_lo | (leaf_hi << 32);
                let mut buf = vec![0u8; self.block_size as usize];
                self.read_block(leaf, &mut buf)?;
                self.walk_extent_node(&buf, out, depth + 1)?;
            }
        }
        Ok(())
    }

    /// Содержимое файла/каталога целиком (без чтения за пределами размера inode).
    fn read_inode_content(&self, inode_num: u32) -> Result<Vec<u8>, FsError> {
        let inode = self.read_inode_data(inode_num)?;
        let size = Self::inode_size_bytes(&inode);
        if size == 0 {
            return Ok(Vec::new());
        }
        if size > MAX_FILE_BYTES {
            return Err(FsError::NotSupported);
        }

        // Быстрый symlink: цель лежит прямо в i_block (до 60 байт).
        let mode = Self::inode_mode(&inode);
        if mode & S_IFMT == S_IFLNK && size < 60 && u32_at(&inode, 28) == 0 {
            return Ok(inode[40..40 + size as usize].to_vec());
        }

        let blocks = self.inode_blocks(&inode, size)?;
        let bs = self.block_size as usize;
        let size = size as usize;
        let mut data = vec![0u8; size];
        let mut buf = vec![0u8; bs];

        for (i, block) in blocks.iter().enumerate() {
            let off = i * bs;
            if off >= size {
                break;
            }
            if *block == 0 {
                continue; // дыра — остаётся нулевой
            }
            self.read_block(*block, &mut buf)?;
            let take = (size - off).min(bs);
            data[off..off + take].copy_from_slice(&buf[..take]);
        }

        Ok(data)
    }

    /// Записи каталога: (имя, номер inode, тип из dirent).
    ///
    /// Читается ВСЁ содержимое каталога. htree-индексные блоки безопасно
    /// пропускаются: их первый fake-dirent имеет inode = 0 и rec_len на весь
    /// блок, поэтому линейный обход не спотыкается о hash-данные.
    fn read_dir_entries(&self, inode_num: u32) -> Result<Vec<DirRaw>, FsError> {
        let data = self.read_inode_content(inode_num)?;
        let mut entries = Vec::new();
        let mut pos = 0usize;

        while pos + 8 <= data.len() {
            let inode = u32_at(&data, pos);
            let rec_len = u16_at(&data, pos + 4) as usize;
            let name_len = data[pos + 6] as usize;
            let file_type = data[pos + 7];

            if rec_len < 8 || pos + rec_len > data.len() {
                break;
            }

            // inode == 0 — удалённая запись, htree-индекс или dir_entry_tail.
            if inode != 0
                && name_len > 0
                && name_len <= 255
                && 8 + name_len <= rec_len
                && pos + 8 + name_len <= data.len()
            {
                let name = core::str::from_utf8(&data[pos + 8..pos + 8 + name_len]).unwrap_or("");
                if !name.is_empty() {
                    entries.push(DirRaw {
                        name: name.to_string(),
                        inode,
                        file_type,
                    });
                }
            }

            pos += rec_len;
        }

        Ok(entries)
    }

    fn find_in_dir(&self, dir_inode: u32, name: &str) -> Result<u32, FsError> {
        for entry in self.read_dir_entries(dir_inode)? {
            if entry.name.eq_ignore_ascii_case(name) {
                return Ok(entry.inode);
            }
        }
        Err(FsError::NotFound)
    }

    fn find_path(&self, path: &str) -> Result<u32, FsError> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(2);
        }

        let mut current_inode = 2u32;
        for part in &parts {
            current_inode = self.find_in_dir(current_inode, part)?;
        }
        Ok(current_inode)
    }

    /// Тип и размер по inode (для readdir/stat без лишних чтений).
    fn entry_meta(&self, inode_num: u32) -> Option<(FileType, u16, u64, u32, u32, u32)> {
        let inode = self.read_inode_data(inode_num).ok()?;
        let mode = Self::inode_mode(&inode);
        let file_type = match mode & S_IFMT {
            S_IFDIR => FileType::Directory,
            S_IFLNK => FileType::Symlink,
            S_IFCHR | S_IFBLK | S_IFIFO | S_IFSOCK => FileType::Device,
            _ => FileType::File,
        };
        let size = Self::inode_size_bytes(&inode);
        let uid = u16_at(&inode, 2) as u32;
        let gid = u16_at(&inode, 24) as u32;
        let mtime = u32_at(&inode, 16);
        Some((file_type, mode & 0o7777, size, uid, gid, mtime))
    }
}

struct Extent {
    logical: u32,
    physical: u64,
    len: u16,
    unwritten: bool,
}

struct DirRaw {
    name: String,
    inode: u32,
    file_type: u8,
}

pub struct Ext2File {
    data: Vec<u8>,
    position: u64,
}

impl File for Ext2File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let pos = self.position as usize;
        if pos >= self.data.len() {
            return Ok(0);
        }
        let to_read = buf.len().min(self.data.len() - pos);
        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }
    fn write(&mut self, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
    }
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, FsError> {
        let new = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::Current(o) => self.position as i64 + o,
            SeekFrom::End(o) => self.data.len() as i64 + o,
        };
        if new < 0 {
            return Err(FsError::InvalidPath);
        }
        self.position = new as u64;
        Ok(self.position)
    }
    fn position(&self) -> u64 {
        self.position
    }
    fn size(&self) -> usize {
        self.data.len()
    }
}

impl FileSystem for Ext2Fs {
    fn name(&self) -> &str {
        self.fs_name
    }

    fn mount(&mut self) -> Result<(), FsError> {
        // Корневой inode уже проверен в `new`; здесь только отмечаем состояние.
        self.mounted = true;
        Ok(())
    }

    fn unmount(&mut self) -> Result<(), FsError> {
        self.mounted = false;
        Ok(())
    }

    fn open(&self, path: &str) -> Result<Box<dyn File>, FsError> {
        let inode_num = self.find_path(path)?;
        let inode = self.read_inode_data(inode_num)?;
        if Self::inode_mode(&inode) & S_IFMT == S_IFDIR {
            return Err(FsError::IsADirectory);
        }

        let data = self.read_inode_content(inode_num)?;
        Ok(Box::new(Ext2File { data, position: 0 }))
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

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let inode_num = self.find_path(path)?;
        let inode = self.read_inode_data(inode_num)?;
        if Self::inode_mode(&inode) & S_IFMT != S_IFDIR {
            return Err(FsError::NotADirectory);
        }

        let raw = self.read_dir_entries(inode_num)?;
        let mut result = Vec::new();

        for entry in raw {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            // Тип берём из dirent (FILETYPE), но размер и права — из inode.
            let (file_type, mode, size, uid, gid) = match self.entry_meta(entry.inode) {
                Some((ft, mode, size, uid, gid, _)) => (ft, mode, size, uid, gid),
                None => {
                    let ft = match entry.file_type {
                        2 => FileType::Directory,
                        7 => FileType::Symlink,
                        _ => FileType::File,
                    };
                    (ft, if ft == FileType::Directory { 0o755 } else { 0o644 }, 0, 0, 0)
                }
            };

            result.push(DirEntry {
                name: entry.name,
                file_type,
                size: if file_type == FileType::Directory {
                    0
                } else {
                    size as usize
                },
                permissions: Permissions::new(mode),
                uid,
                gid,
            });
        }
        Ok(result)
    }

    fn exists(&self, path: &str) -> bool {
        self.find_path(path).is_ok()
    }

    fn stat(&self, path: &str) -> Result<FileMetadata, FsError> {
        let inode_num = self.find_path(path)?;
        let inode = self.read_inode_data(inode_num)?;
        let mode = Self::inode_mode(&inode);
        let file_type = match mode & S_IFMT {
            S_IFDIR => FileType::Directory,
            S_IFLNK => FileType::Symlink,
            S_IFCHR | S_IFBLK | S_IFIFO | S_IFSOCK => FileType::Device,
            _ => FileType::File,
        };
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let name = parts.last().unwrap_or(&"/").to_string();
        let modified = u32_at(&inode, 16) as u64;
        let created = u32_at(&inode, 12) as u64;

        Ok(FileMetadata {
            name,
            file_type,
            size: Self::inode_size_bytes(&inode) as usize,
            permissions: Permissions::new(mode & 0o7777),
            uid: u16_at(&inode, 2) as u32,
            gid: u16_at(&inode, 24) as u32,
            created,
            modified,
        })
    }
}
