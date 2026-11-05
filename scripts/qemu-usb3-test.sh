#!/usr/bin/env bash
# qemu-usb3-test.sh — xHCI на SuperSpeed и hotplug.
#
# Проверяется:
#   1. контроллер xHCI действительно отдаёт USB3-порт (speed 4 = super);
#   2. USB3-накопитель перечисляется и виден в block layer как `sdb`;
#   3. накопитель монтируется и читается через VFS (это и есть сценарий
#      «флешка на ноутбуке должна быть видна»);
#   4. hotplug: отсоединение убирает диск из `disks`, повторное подключение
#      возвращает его без перезагрузки.
#
# QEMU: у `qemu-xhci` первые порты — USB3 (`p3`), поэтому `port=1` даёт
# SuperSpeed. Раньше тесты использовали USB2-порты, и путь USB3 не
# проверялся вообще.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${USB3_TEST_TIMEOUT:-180}"
ready_timeout="${BOOT_READY_TIMEOUT:-90}"

image="$log_dir/hdd.img"
usb_img="$log_dir/usb3-test.img"
serial_log="$log_dir/qemu-usb3.log"

for tool in "$qemu" mkfs.fat mcopy; do
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

echo "==> Preparing a FAT32 volume for the USB3 disk"
rm -f "$usb_img"
truncate -s 64M "$usb_img"
mkfs.fat -F 32 -n USB3TEST "$usb_img" >/dev/null
printf 'hello from usb3\n' > "$log_dir/usb3-hello.txt"
mcopy -i "$usb_img" "$log_dir/usb3-hello.txt" ::/HELLO.TXT

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

echo "==> Booting with an xHCI controller and a SuperSpeed disk"
rm -f "$serial_log"
{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -Fq 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 3

    # Диск и первое монтирование.
    send_text "disks";                 send_text $'\n'; sleep 2
    send_text "mount /mnt auto sdb";   send_text $'\n'; sleep 3
    send_text "cat /mnt/hello.txt";    send_text $'\n'; sleep 2
    send_text "umount /mnt";           send_text $'\n'; sleep 2

    # Hotplug: выдернуть накопитель и вставить снова.
    echo "device_del usbdisk"; sleep 4
    send_text "disks";                 send_text $'\n'; sleep 2
    echo "device_add usb-storage,drive=usbfat2,bus=xhci.0,port=1,id=usbdisk2"
    sleep 5
    send_text "disks";                 send_text $'\n'; sleep 2
    send_text "mount /mnt auto sdb";   send_text $'\n'; sleep 3
    send_text "cat /mnt/hello.txt";    send_text $'\n'; sleep 2
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
    -device "qemu-xhci,id=xhci,p2=4,p3=4" \
    -drive "if=none,id=usbfat,file=$usb_img,format=raw" \
    -device "usb-storage,drive=usbfat,bus=xhci.0,port=1,id=usbdisk" \
    -drive "if=none,id=usbfat2,file=$usb_img,format=raw" \
    || true

fail() {
    echo "usb3 test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 120 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

grep -Fq 'speed 4 = super' "$serial_log" \
    || fail "the controller did not report a SuperSpeed port"
grep -Fq 'mass storage ready' "$serial_log" \
    || fail "USB3 mass storage was not configured"
grep -Fq 'USB QEMU' "$serial_log" \
    || fail "USB3 disk did not reach the block layer"
grep -Fq 'hello from usb3' "$serial_log" \
    || fail "USB3 disk was not readable through the VFS"

# Hotplug: отключение и повторное подключение.
grep -Fq '[xHCI] hotplug: port 1 disconnected' "$serial_log" \
    || fail "hotplug did not notice the disk being removed"
grep -Fq 'block devices refreshed' "$serial_log" \
    || fail "block layer was not refreshed after hotplug"
grep -Fq '[xHCI] hotplug: port 1 connected (speed 4 = super)' "$serial_log" \
    || fail "hotplug did not re-enumerate the disk"
# Второе чтение — уже после повторного подключения.
[[ "$(grep -c 'hello from usb3' "$serial_log")" -ge 2 ]] \
    || fail "disk was not readable after re-plugging"

if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic during USB3 test"
fi
if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message during USB3 test"
fi

echo "usb3 test passed; serial log: $serial_log"
