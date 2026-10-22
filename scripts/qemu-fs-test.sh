#!/usr/bin/env bash
# qemu-fs-test.sh — end-to-end проверка файловых драйверов в QEMU.
#
# Проверяется:
#   1. FAT32 «superfloppy» (диск целиком): mount, ls, cat, LFN.
#   2. ext2: mount, ls, cat, вложенный каталог.
#   3. NTFS (read-only): mount, ls, cat.
#   4. MBR-таблица разделов: определяется sde1, монтируется через `mount auto`.
#
# Требования: qemu-system-x86_64, mtools, e2fsprogs, ntfs-3g, util-linux(sfdisk).
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${FS_TEST_TIMEOUT:-120}"
log_file="$repo_root/build/qemu-fs-test.log"
report="$repo_root/build/fs-test-status.txt"

img_fat="$repo_root/build/fs-test-fat.img"
img_ext="$repo_root/build/fs-test-ext.img"
img_ntfs="$repo_root/build/fs-test-ntfs.img"
img_mbr="$repo_root/build/fs-test-mbr.img"

for tool in "$qemu" mcopy mmd mkfs.fat mke2fs mkntfs ntfscp sfdisk; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done

if [[ ! -f "$repo_root/OVMF.fd" ]]; then
    echo "missing UEFI firmware: $repo_root/OVMF.fd" >&2
    exit 1
fi

# Никаких параллельных QEMU: иначе hdd.img залочен и прогон получается пустым.
pkill -9 -f 'qemu-system[-]x86_64' 2>/dev/null || true
sleep 1

echo "==> Building kernel and boot image"
if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
    make -C "$repo_root" build >/dev/null
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

printf 'hello from fat32\n'   > "$tmp_dir/hello.txt"
printf 'long file name works\n' > "$tmp_dir/long.txt"
printf 'nested ext2 file works\n' > "$tmp_dir/nested.txt"
printf 'hello from ext2\n'    > "$tmp_dir/ext_hello.txt"
printf 'hello from ntfs\n'    > "$tmp_dir/ntfs_hello.txt"
printf 'hello from mbr partition\n' > "$tmp_dir/mbr_hello.txt"

echo "==> Creating test images"

# --- 1. FAT32 superfloppy ---
dd if=/dev/zero of="$img_fat" bs=1M count=32 status=none
mkfs.fat -F 32 -n FSTEST "$img_fat" >/dev/null
mmd -i "$img_fat" ::/docs
mcopy -i "$img_fat" "$tmp_dir/hello.txt" ::/hello.txt
mcopy -i "$img_fat" "$tmp_dir/long.txt" "::/docs/Long File Name.txt"
mcopy -i "$img_fat" "$tmp_dir/long.txt" ::/docs/lfn_test.txt

# --- 2. ext2 (содержимое создаётся через mke2fs -d) ---
mkdir -p "$tmp_dir/ext/sub"
cp "$tmp_dir/ext_hello.txt" "$tmp_dir/ext/hello.txt"
cp "$tmp_dir/nested.txt" "$tmp_dir/ext/sub/nested.txt"
dd if=/dev/zero of="$img_ext" bs=1M count=16 status=none
mke2fs -q -t ext2 -b 1024 -d "$tmp_dir/ext" "$img_ext" >/dev/null

# --- 3. NTFS (read-only) ---
dd if=/dev/zero of="$img_ntfs" bs=1M count=32 status=none
mkntfs -F -q -L NTFSVOL "$img_ntfs" >/dev/null 2>&1
ntfscp -f "$img_ntfs" "$tmp_dir/ntfs_hello.txt" /hello.txt >/dev/null

# --- 4. MBR: один FAT32-раздел с началом на LBA 2048 ---
dd if=/dev/zero of="$img_mbr" bs=1M count=32 status=none
sfdisk --quiet "$img_mbr" >/dev/null <<'EOF'
label: dos
start=2048, size=40960, type=c
EOF
dd if=/dev/zero of="$tmp_dir/part.img" bs=1M count=20 status=none
mkfs.fat -F 32 -n MBRFAT "$tmp_dir/part.img" >/dev/null
mmd -i "$tmp_dir/part.img" ::/mbrdir
mcopy -i "$tmp_dir/part.img" "$tmp_dir/mbr_hello.txt" ::/mbrdir/mbr.txt
dd if="$tmp_dir/part.img" of="$img_mbr" bs=512 seek=2048 conv=notrunc status=none

echo "==> Booting QEMU and driving the shell over serial"
rm -f "$log_file"

# Команды подаём через FIFO: так QEMU не привязан к пайпу и мы можем
# остановить его сразу после появления последнего ожидаемого маркера,
# не дожидаясь таймаута.
serial_in="$tmp_dir/serial.in"
mkfifo "$serial_in"
last_marker='hello from mbr partition'

"$qemu" \
    -m 256M \
    -serial stdio \
    -display none \
    -monitor none \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$repo_root/build/hdd.img,format=raw" \
    -drive "file=$img_fat,format=raw" \
    -drive "file=$img_ext,format=raw" \
    -drive "file=$img_ntfs,format=raw" \
    -drive "file=$img_mbr,format=raw" \
    -M q35 \
    -no-reboot < "$serial_in" > "$log_file" 2>&1 &
qemu_pid=$!

{
    exec 3> "$serial_in"
    sleep 12
    printf 'mount /mnt fat32 sdb\r'        >&3; sleep 2
    printf 'ls /mnt\r'                     >&3; sleep 2
    printf 'cat /mnt/hello.txt\r'          >&3; sleep 2
    printf 'ls /mnt/docs\r'                >&3; sleep 2
    printf 'cat /mnt/docs/lfn_test.txt\r'  >&3; sleep 2

    printf 'mount /mnt2 ext sdc\r'         >&3; sleep 2
    printf 'ls /mnt2\r'                    >&3; sleep 2
    printf 'cat /mnt2/hello.txt\r'         >&3; sleep 2
    printf 'ls /mnt2/sub\r'                >&3; sleep 2
    printf 'cat /mnt2/sub/nested.txt\r'    >&3; sleep 2

    printf 'mount /mnt3 ntfs sdd\r'        >&3; sleep 2
    printf 'ls /mnt3\r'                    >&3; sleep 2
    printf 'cat /mnt3/hello.txt\r'         >&3; sleep 2

    printf 'mount /mnt4 auto sde1\r'       >&3; sleep 2
    printf 'ls /mnt4/mbrdir\r'             >&3; sleep 2
    printf 'cat /mnt4/mbrdir/mbr.txt\r'    >&3; sleep 3

    exec 3>&-
} &
writer_pid=$!

deadline=$((SECONDS + timeout_seconds))
while kill -0 "$qemu_pid" 2>/dev/null; do
    if grep -Fq "$last_marker" "$log_file" 2>/dev/null; then
        sleep 1
        break
    fi
    if (( SECONDS >= deadline )); then
        break
    fi
    sleep 1
done

kill -TERM "$qemu_pid" 2>/dev/null || true
sleep 1
kill -KILL "$qemu_pid" 2>/dev/null || true
kill -KILL "$writer_pid" 2>/dev/null || true
wait "$qemu_pid" 2>/dev/null || true
wait "$writer_pid" 2>/dev/null || true

failed=0
: > "$report"

check() {
    local marker="$1"
    if grep -Fq "$marker" "$log_file"; then
        echo "  [ OK ] $marker" | tee -a "$report"
    else
        echo "  [FAIL] $marker" | tee -a "$report" >&2
        failed=1
    fi
}

echo "==> Checking results"
check "OK (fat32)"
check "hello from fat32"
check "Long File Name.txt"
check "long file name works"

check "OK (ext, ro)"
check "hello from ext2"
check "nested ext2 file works"

check "OK (ntfs, ro)"
check "hello from ntfs"

check "MBR.TXT"
check "hello from mbr partition"

if grep -Fq "PANIC" "$log_file"; then
    echo "  [FAIL] kernel panic detected" | tee -a "$report" >&2
    failed=1
fi

if [[ "$failed" -ne 0 ]]; then
    echo "FS test failed; serial log: $log_file" >&2
    tail -60 "$log_file" >&2
    exit 1
fi

echo "FS test passed; serial log: $log_file"
