#!/usr/bin/env bash
# qemu-iso-test.sh — чтение ISO9660 с CD-ROM через ATAPI (`sr0`).
#
# До этого `srX` был просто блочным устройством: диск, с которого ядро
# грузится, нельзя было даже посмотреть. Проверяется:
#   * `disks` определяет ISO9660 по PVD в LBA 16;
#   * `mount auto sr0` монтирует только для чтения;
#   * имена Joliet/Rock Ridge (нижний регистр, длинные имена);
#   * файл в подкаталоге и размер extent'а (файл не влезает в один блок).
#
# Требования: qemu-system-x86_64, genisoimage.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${ISO_TEST_TIMEOUT:-150}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"

image="$log_dir/hdd.img"
iso="$log_dir/iso-test.iso"
serial_log="$log_dir/qemu-iso.log"
src_dir="$log_dir/iso-test-src"

for tool in "$qemu" genisoimage; do
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

echo "==> Preparing ISO9660 image"
rm -rf "$src_dir"
mkdir -p "$src_dir/docs"
printf 'hello from iso9660\n' > "$src_dir/hello.txt"
printf 'nested iso file\n' > "$src_dir/docs/nested.txt"
# 100 КиБ: extent больше одного 2048-байтного блока, размер проверяем через `ls -l`.
head -c 102400 /dev/zero | tr '\0' 'B' > "$src_dir/docs/blob.bin"
genisoimage -quiet -R -J -V ISOVOL -o "$iso" "$src_dir"

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '/') echo "slash" ;;
        '.') echo "dot" ;;
        '-') echo "minus" ;;
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

echo "==> Booting QEMU with the ISO attached as CD-ROM"
rm -f "$serial_log"
{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -Fq 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 2

    send_text "disks";                send_text $'\n'; sleep 2
    send_text "mount /mnt auto sr0";  send_text $'\n'; sleep 3
    send_text "ls /mnt";              send_text $'\n'; sleep 3
    send_text "cat /mnt/hello.txt";   send_text $'\n'; sleep 2
    send_text "cat /mnt/docs/nested.txt"; send_text $'\n'; sleep 2
    send_text "ls -l /mnt/docs";      send_text $'\n'; sleep 2
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
    "$qemu" \
    -m 512M \
    -serial "file:$serial_log" \
    -monitor stdio \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw,snapshot=on" \
    -M q35 \
    -display none \
    -no-reboot \
    -drive "if=none,id=cd,file=$iso,format=raw,media=cdrom" \
    -device "ide-cd,drive=cd,bus=ide.1" || true

fail() {
    echo "iso9660 test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 120 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

grep -Eq '[[:space:]]iso9660' "$serial_log" || fail "disks did not detect ISO9660"
grep -Eq '[[:space:]]ISOVOL' "$serial_log" || fail "disks did not show the ISO label"
grep -Fq 'OK (iso9660, ro)' "$serial_log" || fail "ISO9660 did not mount"
grep -Fq 'hello from iso9660' "$serial_log" || fail "file in the root was not read"
grep -Fq 'nested iso file' "$serial_log" || fail "file in a subdirectory was not read"
# Joliet/Rock Ridge: имя в нижнем регистре без `;1`.
grep -Fq 'hello.txt' "$serial_log" || fail "lowercase name was not decoded"
grep -Fq 'docs/' "$serial_log" || fail "subdirectory was not listed"
# Размер extent'а из directory record (100 КиБ > одного блока).
grep -Eq '102400' "$serial_log" || fail "extent size was not reported"

if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic while reading ISO9660"
fi
if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message while reading ISO9660"
fi

echo "iso9660 test passed; serial log: $serial_log"
