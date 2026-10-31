---
name: Vibra Filesystems
description: Work on Vibra filesystem and block-layer code — ext2/3/4, FAT32, NTFS, MBR/EBR/GPT, 4Kn, debug images and filesystem tests.
---

# Vibra Filesystems

## Units contract

- `DiskIo::read(sector, buf)` always uses **512-byte units**, even on 4Kn.
- `DiskIo::read_lba(lba, buf)` uses **device logical blocks**
  (`sector_size()`: 512 or 4096).
- `PartitionDisk::offset/sectors` are always 512-byte units.
- GPT stores LBA in device blocks; convert with `ss / 512` before creating a
  `Partition`.

Violating this contract makes a partition window drift (on 4Kn it lands 8×
further) and the FS reads as `unknown`.

## Drivers

| File | Supports | Write |
|---|---|---|
| `fs/fat32.rs` | FAT32, LFN, mkdir/rmdir | yes (`new_rw`, explicit `rw`) |
| `fs/ext2.rs` | ext2, ext3, ext4 (extents, 64bit, flex_bg, dir_index, metadata_csum) | no |
| `fs/ntfs.rs` | NTFS read via MFT/index | no |
| `fs/partition.rs` | MBR + EBR chain + GPT CRC | — |
| `fs/blockdev.rs` | sda/sdb + sr0 ATAPI registration | — |

ext4 rules:

- Reject unsupported incompat features (`inline_data`, `encrypt`, `bigalloc`,
  `meta_bg`, `compression`) with `FsError::NotSupported`.
- Never report a successful mount if the root inode cannot be read.
- Extent leaf/index entries are 12 bytes; header magic is `0xF30A`.
- 64bit group descriptors are `s_desc_size` bytes, inode table high half at
  offset `0x28`.
- htree directories are safe to scan linearly: fake dirents have `inode = 0`.

## Tests

```sh
make fs-test          # FAT32/ext2/ext4/NTFS + MBR via serial
make debug-image-test # 3-partition USB image, VIBRALOG write-back
make iso-debug-test   # debug ISO as USB
```

`scripts/qemu-fs-test.sh` creates images with `mkfs.fat`, `mke2fs -t ext2`,
`mke2fs -t ext4` (default extents/64bit/metadata_csum), `mkntfs` and `sfdisk`.

## Debug images

`make usb-debug` → GPT: `VIBRA-ESP`, `VIBRA` (kernel), `VIBRALOG`.
`make iso-debug` → hybrid ISO with appended `VIBRA` and `VIBRALOG`.
Limine loads the kernel via `fslabel(VIBRA):/kernel.elf`.
CD/DVD is read-only; the kernel logs a warning instead of pretending to write.
