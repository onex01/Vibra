#!/usr/bin/env python3
"""Создаёт минимальный exFAT-образ для тестов драйвера (только чтение).

`mkfs.exfat` умеет только форматировать том, а положить в него файлы без
root и loop-устройства нечем. Поэтому фикстура собирается вручную:

  * 512 байт на сектор, 8 секторов на кластер (4 КиБ);
  * корневой каталог — кластер 2;
  * `hello.txt` — один contiguous-кластер (`NoFatChain`);
  * `sub/nested.txt` — подкаталог + файл в нём;
  * `big.txt` — три кластера, связанные через FAT (`NoFatChain` снят),
    чтобы проверялось чтение через границу кластера.

Использование: scripts/mk-exfat-fixture.py <image> [size-mb]
"""

import struct
import sys

SECTOR = 512
SPC = 8                       # секторов на кластер
CLUSTER = SECTOR * SPC
FAT_OFFSET = 128              # в секторах
HEAP_OFFSET = 2048            # в секторах
ROOT_CLUSTER = 2


def name_entries(name: str) -> list[bytes]:
    """Имя файла: одна или несколько записей 0xC1 по 15 символов UTF-16LE."""
    units = name.encode("utf-16-le")
    chars = [units[i:i + 2] for i in range(0, len(units), 2)]
    out = []
    for i in range(0, len(chars), 15):
        chunk = chars[i:i + 15]
        entry = bytearray(32)
        entry[0] = 0xC1
        for j, ch in enumerate(chunk):
            entry[2 + j * 2:4 + j * 2] = ch
        out.append(bytes(entry))
    return out


def entry_set(name: str, first_cluster: int, size: int, is_dir: bool,
              no_fat_chain: bool) -> bytes:
    """0x85 (File) + 0xC0 (Stream) + 0xC1… (Name)."""
    names = name_entries(name)
    file_entry = bytearray(32)
    file_entry[0] = 0x85
    file_entry[1] = 1 + len(names)               # secondary count
    # checksum намеренно нулевой: драйвер только читает, а реальный том
    # проверяется отдельными тестами на настоящем mkfs.exfat.
    file_entry[4:6] = struct.pack("<H", 0x10 if is_dir else 0x20)

    stream = bytearray(32)
    stream[0] = 0xC0
    stream[1] = 0x01 | (0x02 if no_fat_chain else 0x00)
    stream[3] = len(name.encode("utf-16-le")) // 2 & 0xFF
    stream[8:16] = struct.pack("<Q", size)
    stream[20:24] = struct.pack("<I", first_cluster)
    stream[24:32] = struct.pack("<Q", size)

    return bytes(file_entry) + bytes(stream) + b"".join(names)


def label_entry(label: str) -> bytes:
    entry = bytearray(32)
    entry[0] = 0x83
    units = label.encode("utf-16-le")[:22]
    entry[1] = len(units) // 2
    entry[2:2 + len(units)] = units
    return bytes(entry)


def make_volume_label_boot_sector(total_sectors: int, fat_length: int,
                                  cluster_count: int) -> bytes:
    bs = bytearray(SECTOR)
    bs[0:3] = b"\xEB\x76\x90"
    bs[3:11] = b"EXFAT   "
    bs[64:72] = struct.pack("<Q", 0)                 # PartitionOffset
    bs[72:80] = struct.pack("<Q", total_sectors)     # VolumeLength
    bs[80:84] = struct.pack("<I", FAT_OFFSET)        # FatOffset
    bs[84:88] = struct.pack("<I", fat_length)        # FatLength
    bs[88:92] = struct.pack("<I", HEAP_OFFSET)       # ClusterHeapOffset
    bs[92:96] = struct.pack("<I", cluster_count)     # ClusterCount
    bs[96:100] = struct.pack("<I", ROOT_CLUSTER)     # FirstClusterOfRootDirectory
    bs[100:104] = struct.pack("<I", 0x45584641)      # VolumeSerialNumber
    bs[104:106] = struct.pack("<H", 0x0100)          # FileSystemRevision 1.0
    bs[106:108] = struct.pack("<H", 0)               # VolumeFlags
    bs[108] = 9                                       # BytesPerSectorShift -> 512
    bs[109] = 3                                       # SectorsPerClusterShift -> 8
    bs[110] = 1                                       # NumberOfFats
    bs[111] = 0x80                                    # DriveSelect
    bs[112] = 0                                       # PercentInUse
    bs[510:512] = b"\x55\xAA"
    return bytes(bs)


def boot_checksum(bs: bytes) -> int:
    """Checksum boot-сектора: байты 106..112 считаются нулевыми."""
    data = bytearray(bs)
    for i in range(106, 112):
        data[i] = 0
    value = 0
    for b in data:
        value = (((value << 31) | (value >> 1)) + b) & 0xFFFFFFFF
    return value


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    path = sys.argv[1]
    size_mb = int(sys.argv[2]) if len(sys.argv) > 2 else 256

    total_sectors = size_mb * 1024 * 1024 // SECTOR
    cluster_count = (total_sectors - HEAP_OFFSET) // SPC
    fat_length = ((cluster_count + 2) * 4 + SECTOR - 1) // SECTOR
    assert FAT_OFFSET + fat_length <= HEAP_OFFSET, "FAT не влезает до heap"

    fat = bytearray(fat_length * SECTOR)

    def set_fat(cluster: int, value: int) -> None:
        off = cluster * 4
        fat[off:off + 4] = struct.pack("<I", value)

    def cluster_offset(cluster: int) -> int:
        return (HEAP_OFFSET + (cluster - 2) * SPC) * SECTOR

    # --- раскладка кластеров ---
    # 2 — корень, 3 — hello.txt, 4 — sub, 5 — sub/nested.txt,
    # 6,7 — big.txt (FAT-цепочка; 1.5 кластера).
    root_cluster, hello_cluster, sub_cluster = 2, 3, 4
    nested_cluster, big_start = 5, 6

    hello = b"hello from exfat\n"
    nested = b"nested exfat file\n"
    # Содержимое различается по кластерам: так проверяется порядок цепочки,
    # а не только длина. 1.5 кластера — чтение пересекает границу.
    big = b"A" * CLUSTER + b"B" * (CLUSTER // 2)

    set_fat(hello_cluster, 0xFFFFFFFF)   # EOC
    set_fat(sub_cluster, 0xFFFFFFFF)
    set_fat(nested_cluster, 0xFFFFFFFF)
    # big.txt: 6 -> 7 -> EOC
    set_fat(big_start, big_start + 1)
    set_fat(big_start + 1, 0xFFFFFFFF)

    # --- корневой каталог ---
    root = bytearray(CLUSTER)
    off = 0
    for chunk in (
        label_entry("EXFATVOL"),
        entry_set("hello.txt", hello_cluster, len(hello), False, True),
        entry_set("sub", sub_cluster, CLUSTER, True, False),
        entry_set("big.txt", big_start, len(big), False, False),
    ):
        root[off:off + len(chunk)] = chunk
        off += len(chunk)

    sub = bytearray(CLUSTER)
    chunk = entry_set("nested.txt", nested_cluster, len(nested), False, True)
    sub[0:len(chunk)] = chunk

    bs = make_volume_label_boot_sector(total_sectors, fat_length, cluster_count)
    # Main Boot Region: сектор 0 — boot sector, 1–8 — extended boot, 9 — OEM,
    # 10 — reserved, 11 — checksum всего региона. Драйверу checksum не нужен,
    # но с ним образ читается и другими ОС.
    checksum = boot_checksum(bs)

    with open(path, "wb") as f:
        f.truncate(total_sectors * SECTOR)
        f.seek(0)
        f.write(bs)
        f.seek(11 * SECTOR)
        f.write(struct.pack("<I", checksum))
        f.seek(FAT_OFFSET * SECTOR)
        f.write(bytes(fat))
        f.seek(cluster_offset(root_cluster))
        f.write(bytes(root))
        f.seek(cluster_offset(hello_cluster))
        f.write(hello)
        f.seek(cluster_offset(sub_cluster))
        f.write(bytes(sub))
        f.seek(cluster_offset(nested_cluster))
        f.write(nested)
        f.seek(cluster_offset(big_start))
        f.write(big)

    print(f"exfat fixture: {path} ({size_mb} MiB, {cluster_count} clusters)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
