#!/usr/bin/env bash
# qemu-usb-storage-test.sh — USB mass storage через VFS.
#
# Проверяется весь путь: xHCI → BOT/SCSI → DiskIo → DiskManager (sdb) →
# mount fat32 → ls/cat → запись (`cp` внутри тома) → проверка на хосте.
#
# Требует собранный hdd.img (`make build`) и OVMF.fd.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
usb_img="$log_dir/usb-storage-test.img"
serial_log="$log_dir/qemu-usb-storage-test.log"
hello="$log_dir/usb-storage-hello.txt"
timeout_seconds="${USB_STORAGE_TEST_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"

for tool in qemu-system-x86_64 mkfs.fat mcopy mtype mdir; do
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

echo "==> Preparing USB disk (64 MiB FAT32, label VIBRADATA)"
rm -f "$usb_img"
dd if=/dev/zero of="$usb_img" bs=1M count=64 status=none
mkfs.fat -F 32 -n VIBRADATA "$usb_img" >/dev/null
printf 'hello from usb\n' > "$hello"
mcopy -i "$usb_img" "$hello" ::/HELLO.TXT
mdir -i "$usb_img" ::/ | sed 's/^/    /'

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '-') echo "minus" ;;
        '.') echo "dot" ;;
        '/') echo "slash" ;;
        # Верхний регистр требует shift: иначе sendkey молча теряет символ.
        [A-Z]) printf 'shift-%s\n' "$(printf '%s' "$1" | tr 'A-Z' 'a-z')" ;;
        *) echo "$1" ;;
    esac
}

send_text() {
    local text="$1" ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.04
    done
}

echo "==> Booting with usb-storage attached"
rm -f "$serial_log"
{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -q 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done

    send_text "mount /mnt fat32 sdb rw"
    send_text $'\n'
    sleep 2
    send_text "ls /mnt"
    send_text $'\n'
    sleep 1
    send_text "cat /mnt/hello.txt"
    send_text $'\n'
    sleep 1
    send_text "cp /mnt/hello.txt /mnt/copy.txt"
    send_text $'\n'
    sleep 2
    send_text "ls /mnt"
    send_text $'\n'
    sleep 1
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
    qemu-system-x86_64 \
    -m 256M \
    -serial "file:$serial_log" \
    -monitor stdio \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw" \
    -M q35 \
    -display none \
    -no-reboot \
    -device nec-usb-xhci,id=xhci \
    -drive if=none,id=usbdisk,file="$usb_img",format=raw \
    -device usb-storage,drive=usbdisk,bus=xhci.0 >/dev/null 2>"$log_dir/qemu-usb-storage-stderr.log" || true

if [[ -s "$log_dir/qemu-usb-storage-stderr.log" ]]; then
    echo "  qemu stderr:"
    sed 's/^/    /' "$log_dir/qemu-usb-storage-stderr.log"
fi

fail=0
check_serial() {
    if grep -qa -- "$1" "$serial_log"; then
        echo "  ok: $2"
    else
        echo "  FAIL: $2" >&2
        fail=1
    fi
}

check_serial '\[USB-STORAGE\].*QEMU HARDDISK' 'xHCI enumerated USB mass storage'
check_serial '\[BLK\] sdb' 'USB disk registered as sdb'
check_serial 'hello from usb' 'cat /mnt/hello.txt returned file contents'
check_serial 'Copied: /mnt/hello.txt -> /mnt/copy.txt' 'cp wrote into the mounted volume'

echo "==> Verifying write-back with mtools"
if mtype -i "$usb_img" ::/COPY.TXT 2>/dev/null | grep -qa 'hello from usb'; then
    echo "  ok: /COPY.TXT on the USB image contains the copied data"
else
    echo "  FAIL: /COPY.TXT missing or wrong on the USB image" >&2
    fail=1
fi

if command -v fsck.fat >/dev/null 2>&1; then
    if fsck.fat -n "$usb_img" >/dev/null 2>&1; then
        echo "  ok: FAT32 volume is consistent (fsck.fat)"
    else
        echo "  FAIL: fsck.fat reported errors" >&2
        fsck.fat -n "$usb_img" >&2 || true
        fail=1
    fi
fi

if [[ "$fail" -ne 0 ]]; then
    echo "--- serial tail ---" >&2
    tail -40 "$serial_log" >&2 || true
    exit 1
fi

echo "==> USB storage test passed"
