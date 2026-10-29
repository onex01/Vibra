#!/usr/bin/env bash
# qemu-bad-fs-test.sh — повреждённые образы не должны ронять ядро.
#
# Подключаются четыре заведомо битых образа:
#   * FAT32 с нулевым boot sector;
#   * ext2 с нулевым суперблоком;
#   * NTFS с нулевым boot sector;
#   * MBR, у которого раздел начинается далеко за концом диска.
#
# Ожидается: каждый `mount` печатает FAILED, shell остаётся жив, `ls /`
# продолжает работать, в логе нет KERNEL PANIC / [FATAL].
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
serial_log="$log_dir/qemu-bad-fs.log"
timeout_seconds="${BAD_FS_TEST_TIMEOUT:-180}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

for tool in "$QEMU" python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done
for file in "$image" "$repo_root/OVMF.fd"; do
    if [[ ! -f "$file" ]]; then
        echo "missing $file (run ./build.sh setup && make build)" >&2
        exit 1
    fi
done

bad_fat="$log_dir/bad-fat.img"
bad_ext="$log_dir/bad-ext.img"
bad_ntfs="$log_dir/bad-ntfs.img"
bad_mbr="$log_dir/bad-mbr.img"

echo "==> Creating corrupt images"
dd if=/dev/zero of="$bad_fat"  bs=1M count=1  status=none
dd if=/dev/zero of="$bad_ext"  bs=1M count=2  status=none
dd if=/dev/zero of="$bad_ntfs" bs=1M count=2  status=none
dd if=/dev/zero of="$bad_mbr"  bs=1M count=4  status=none

python3 - "$bad_mbr" <<'PY'
import struct
import sys

path = sys.argv[1]
sector = bytearray(512)
# MBR signature + одна запись: тип 0x0C, раздел внутри диска, но без ФС.
sector[446 + 4] = 0x0C
sector[446 + 8:446 + 12] = struct.pack("<I", 2048)
sector[446 + 12:446 + 16] = struct.pack("<I", 2048)
sector[510] = 0x55
sector[511] = 0xAA
with open(path, "r+b") as f:
    f.write(sector)
PY

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '/') echo "slash" ;;
        '.') echo "dot" ;;
        '-') echo "minus" ;;
        *) echo "$1" ;;
    esac
}

send_text() {
    local text="$1" ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.10
    done
}

rm -f "$serial_log"
{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -q 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 3

    send_text "mount /mnt/fat fat32 sdb";  send_text $'\n'; sleep 2
    send_text "mount /mnt/ext ext sdc";    send_text $'\n'; sleep 2
    send_text "mount /mnt/ntfs ntfs sdd";  send_text $'\n'; sleep 2
    send_text "mount /mnt/mbr auto sde1";  send_text $'\n'; sleep 2
    send_text "ls /";                      send_text $'\n'; sleep 1
    send_text "cat /cfg/hostname";         send_text $'\n'; sleep 1
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
    "$QEMU" \
    -m 256M \
    -serial "file:$serial_log" \
    -monitor stdio \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw" \
    -drive "file=$bad_fat,format=raw" \
    -drive "file=$bad_ext,format=raw" \
    -drive "file=$bad_ntfs,format=raw" \
    -drive "file=$bad_mbr,format=raw" \
    -M q35 \
    -display none \
    -no-reboot || true

fail() {
    echo "bad-fs test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 200 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

failures="$(grep -c 'FAILED' "$serial_log" || true)"
if [[ "${failures:-0}" -lt 4 ]]; then
    fail "expected 4 mount failures, got ${failures:-0}"
fi

# Shell и корневая ФС живы после каждой ошибки.
grep -Fq 'vibra' "$serial_log" || fail "hostname unreadable after bad mounts"
for dir in dev proc sys cfg home var tmp mnt; do
    grep -Eq "(^| )${dir}/?( |$)" "$serial_log" || fail "ls / lost ${dir} after bad mounts"
done

if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic on corrupt filesystem"
fi
if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message on corrupt filesystem"
fi

echo "bad-fs test passed; serial log: $serial_log"
