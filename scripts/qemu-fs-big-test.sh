#!/usr/bin/env bash
# qemu-fs-big-test.sh — большие тома и 4Kn: NTFS 2 ТБ, GPT за 2 ТБ, MBR 4Kn.
#
# В одном прогоне QEMU подключаются три образа:
#   sdb — NTFS 2 ТБ целиком (superfloppy, без таблицы разделов);
#   sdc — GPT 3 ТБ, раздел начинается на LBA 4 296 873 984 (~2.2 ТБ);
#   sdd — MBR на 4Kn-устройстве (4096-байтные сектора), NTFS 512 МиБ.
#
# Проверяется то, что ломалось раньше:
#   * GPT хранит LBA в логических секторах устройства — на 4Kn их надо
#     переводить в 512-байтные единицы;
#   * MBR-записи тоже заданы в секторах устройства, а не в 512-байтных;
#   * чтение NTFS больше 64 МиБ идёт потоково (`cat` не падает с OOM);
#   * `mount auto` монтирует большой том, а не отвечает `unknown`.
#
# Образы создаются разреженными (`truncate`), поэтому 2–3 ТБ не занимают
# места на диске хоста: реально пишутся только метаданные ФС.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
hdd="$log_dir/hdd.img"
serial_log="$log_dir/qemu-fs-big.log"
serial_4k="$log_dir/qemu-fs-big-4k.log"
timeout_seconds="${BIG_FS_TEST_TIMEOUT:-180}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

big2t="$log_dir/fs-big-2t.img"
fat16="$log_dir/fs-big-fat16.img"
labelfs="$log_dir/fs-big-label.img"
gpt3t="$log_dir/fs-big-gpt3t.img"
mbr4k="$log_dir/fs-big-mbr4k.img"

for tool in "$QEMU" mkntfs ntfscp mcopy mkfs.fat python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done
for file in "$hdd" "$repo_root/OVMF.fd"; do
    if [[ ! -f "$file" ]]; then
        echo "missing $file (run ./build.sh setup && make build)" >&2
        exit 1
    fi
done

cleanup() {
    rm -f "$big2t" "$gpt3t" "$mbr4k" "$fat16" "$labelfs"
}
trap cleanup EXIT

echo "==> Preparing NTFS 2T (whole disk)"
truncate -s 2T "$big2t"
mkntfs -F -f -Q -L BIG2T "$big2t" >/dev/null 2>&1
printf 'hello from 2TB ntfs\n' > "$log_dir/fs-big-2t.txt"
ntfscp -f "$big2t" "$log_dir/fs-big-2t.txt" /HELLO.TXT

echo "==> Preparing GPT 3T with a partition above 2TB"
truncate -s 3T "$gpt3t"
# Раздел 2.2 ТБ … 2.328 ТБ — LBA больше 2^32/2, проверяет 64-битную арифметику.
python3 - "$gpt3t" <<'PY'
import struct
import sys
import uuid
import zlib

path = sys.argv[1]
size = 3 * 1024 ** 4
ss = 512
total = size // ss
img = open(path, "r+b")

mbr = bytearray(512)
mbr[446 + 4] = 0xEE
mbr[446 + 8:446 + 12] = struct.pack("<I", 1)
mbr[446 + 12:446 + 16] = struct.pack("<I", min(total - 1, 0xFFFFFFFF))
mbr[510] = 0x55
mbr[511] = 0xAA
img.seek(0)
img.write(mbr)

first = 4296873984
last = first + 262144 - 1  # 128 МиБ в 512-байтных секторах
entries = bytearray(128 * 128)
e = bytearray(128)
e[0:16] = bytes.fromhex("A2A0D0EBE5B9334487C068B6B72699C7")
e[16:32] = uuid.uuid4().bytes_le
e[32:40] = struct.pack("<Q", first)
e[40:48] = struct.pack("<Q", last)
entries[0:128] = e
entries_crc = zlib.crc32(entries) & 0xFFFFFFFF
img.seek(2 * ss)
img.write(entries)

hdr = bytearray(92)
hdr[0:8] = b"EFI PART"
hdr[8:12] = struct.pack("<I", 0x00010000)
hdr[12:16] = struct.pack("<I", 92)
hdr[24:32] = struct.pack("<Q", 1)
hdr[32:40] = struct.pack("<Q", total - 1)
hdr[40:48] = struct.pack("<Q", 6)
hdr[48:56] = struct.pack("<Q", total - 34)
hdr[56:72] = uuid.uuid4().bytes_le
hdr[72:80] = struct.pack("<Q", 2)
hdr[80:84] = struct.pack("<I", 128)
hdr[84:88] = struct.pack("<I", 128)
hdr[88:92] = struct.pack("<I", entries_crc)
hdr[16:20] = struct.pack("<I", zlib.crc32(hdr) & 0xFFFFFFFF)
img.seek(1 * ss)
img.write(hdr)
img.close()
PY
truncate -s 128M "$log_dir/fs-big-part.img"
mkntfs -F -f -Q -L GPTBIG "$log_dir/fs-big-part.img" >/dev/null 2>&1
printf 'hello from gpt above 2TB\n' > "$log_dir/fs-big-gpt.txt"
ntfscp -f "$log_dir/fs-big-part.img" "$log_dir/fs-big-gpt.txt" /HELLO.TXT
dd if="$log_dir/fs-big-part.img" of="$gpt3t" bs=512 seek=4296873984 conv=notrunc status=none

echo "==> Preparing MBR on 4Kn device (4096-byte sectors)"
truncate -s 512G "$mbr4k"
truncate -s 512M "$log_dir/fs-big-part4k.img"
mkntfs -F -f -Q -s 4096 -c 4096 -L MBR4K "$log_dir/fs-big-part4k.img" >/dev/null 2>&1
printf 'hello from mbr 4kn\n' > "$log_dir/fs-big-mbr.txt"
ntfscp -f "$log_dir/fs-big-part4k.img" "$log_dir/fs-big-mbr.txt" /HELLO.TXT
python3 - "$mbr4k" "$log_dir/fs-big-part4k.img" <<'PY'
import struct
import sys

image, part = sys.argv[1], sys.argv[2]
mbr = bytearray(512)
# MBR-поля — в 4096-байтных секторах устройства: start=2048 → 8 МиБ.
mbr[446 + 4] = 0x07
mbr[446 + 8:446 + 12] = struct.pack("<I", 2048)
mbr[446 + 12:446 + 16] = struct.pack("<I", 131072)  # 512 МиБ
mbr[510] = 0x55
mbr[511] = 0xAA
with open(image, "r+b") as f:
    f.seek(0)
    f.write(mbr)
    f.seek(2048 * 4096)
    with open(part, "rb") as p:
        while True:
            chunk = p.read(1 << 20)
            if not chunk:
                break
            f.write(chunk)
PY

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '/') echo "slash" ;;
        '.') echo "dot" ;;
        '-') echo "minus" ;;
        '=') echo "equal" ;;
        [A-Z]) printf 'shift-%s\n' "$(printf '%s' "$1" | tr 'A-Z' 'a-z')" ;;
        *) echo "$1" ;;
    esac
}
send_text() {
    local text="$1" ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.06
    done
}

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '/') echo "slash" ;;
        '.') echo "dot" ;;
        '-') echo "minus" ;;
        '=') echo "equal" ;;
        [A-Z]) printf 'shift-%s\n' "$(printf '%s' "$1" | tr 'A-Z' 'a-z')" ;;
        *) echo "$1" ;;
    esac
}
send_text() {
    local text="$1" ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.06
    done
}

fail() {
    echo "big filesystem test failed: $1" >&2
    for log in "$serial_log" "$serial_4k"; do
        [[ ! -f "$log" ]] || tail -n 120 "$log" >&2
    done
    exit 1
}

# Один прогон QEMU: ждём prompt, подаём команды, проверяем маркеры.
run_case() {
    local log="$1" expect="$2"; shift 2
    # До "--" идут команды shell, после — дополнительные аргументы QEMU.
    local cmds=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do
        cmds+=("$1")
        shift
    done
    [[ "${1:-}" == "--" ]] && shift
    rm -f "$log"
    {
        for _ in $(seq 1 "$ready_timeout"); do
            if grep -q 'vibra:/>' "$log" 2>/dev/null; then
                break
            fi
            sleep 1
        done
        # Даём дискам доинициализироваться: NVMe/IDE отвечают несколько
        # медленнее, и первые клавиши иначе теряются.
        sleep 4
        for cmd in "${cmds[@]}"; do
            send_text "$cmd"
            send_text $'\n'
            sleep 3
        done
        echo "quit"
    } | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
        "$QEMU" -m 768M \
        -serial "file:$log" \
        -monitor stdio \
        -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
        -drive "file=$hdd,format=raw,snapshot=on" \
        -M q35 -display none -no-reboot \
        "$@" || true

    [[ -f "$log" ]] || fail "no serial log for $expect"
    grep -Fq "$expect" "$log" || fail "$expect was not mounted/read"
    if grep -Fq 'KERNEL PANIC' "$log"; then
        fail "kernel panic while checking $expect"
    fi
    if grep -Fq '[FATAL]' "$log"; then
        fail "fatal message while checking $expect"
    fi
}

echo "==> Preparing FAT32 volume with a label (mount by LABEL=)"
truncate -s 64M "$labelfs"
mkfs.fat -F 32 -n LABELVOL "$labelfs" >/dev/null
printf 'hello from label\n' > "$log_dir/fs-big-label.txt"
mcopy -i "$labelfs" "$log_dir/fs-big-label.txt" ::/LABEL.TXT

echo "==> Preparing FAT16 volume (must be detected, not mounted)"
truncate -s 64M "$fat16"
mkfs.fat -F 16 -n FSTEST16 "$fat16" >/dev/null

echo "==> Case 1: 2T NTFS whole disk + GPT partition above 2T (SATA)"
run_case "$serial_log" 'hello from 2TB ntfs' \
    "disks -v" \
    "mount /mnt auto sdb" \
    "cat /mnt/hello.txt" \
    "mount /mnt2 auto sdc1" \
    "cat /mnt2/hello.txt" \
    "mount /mnt3 auto sdd" \
    "umount /mnt" \
    "umount /mnt2" \
    -- \
    -drive "if=none,id=big2t,file=$big2t,format=raw" \
    -device "ide-hd,drive=big2t,bus=ide.1" \
    -drive "if=none,id=gpt3t,file=$gpt3t,format=raw" \
    -device "ide-hd,drive=gpt3t,bus=ide.2" \
    -drive "if=none,id=fat16,file=$fat16,format=raw" \
    -device "ide-hd,drive=fat16,bus=ide.3"

grep -Fq 'hello from gpt above 2TB' "$serial_log" \
    || fail "GPT partition above 2TB was not mounted/read"
grep -Eq '[[:space:]]gpt[[:space:]]*$' "$serial_log" \
    || fail "disks did not report the GPT table"
grep -Eq '[[:space:]]fat16[[:space:]]*$' "$serial_log" \
    || fail "disks did not detect FAT16"
grep -Fq 'SKIPPED (FAT12/FAT16 is not supported; only FAT32)' "$serial_log" \
    || fail "mount did not explain that FAT16 is unsupported"
grep -Fq 'Unmounted /mnt2' "$serial_log" || fail "umount did not release /mnt2"
grep -Eq 'driver: (ahci|ide)' "$serial_log" || fail "disks -v has no driver column"
grep -Fq 'model:' "$serial_log" || fail "disks -v did not report a device model"

echo "==> Case 2: MBR on a 4Kn device (NVMe, 4096-byte sectors)"
run_case "$serial_4k" 'hello from mbr 4kn' \
    "disks -v" \
    "mount /mnt auto sdb1" \
    "cat /mnt/hello.txt" \
    -- \
    -drive "if=none,id=mbr4k,file=$mbr4k,format=raw" \
    -device "nvme,drive=mbr4k,serial=MBR4K,logical_block_size=4096,physical_block_size=4096"

grep -Eq '[[:space:]]mbr[[:space:]]*$' "$serial_4k" \
    || fail "disks did not report the MBR table"
grep -Fq 'driver: nvme' "$serial_4k" || fail "disks -v has no driver column"
grep -Fq 'sector: 4096 B' "$serial_4k" || fail "disks -v did not report the 4Kn sector size"

echo "big filesystem test passed; logs: $serial_log, $serial_4k"
