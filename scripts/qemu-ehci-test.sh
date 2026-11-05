#!/usr/bin/env bash
# qemu-ehci-test.sh — EHCI (USB 2.0) и hotplug.
#
# Зачем: на Haswell/Broadwell USB2-половина USB3-разъёмов принадлежит EHCI,
# поэтому без этого драйвера флешка на ноутбуке не видна. QEMU умеет
# `usb-ehci` + `usb-storage`, значит путь можно проверить без железа.
#
# Проверяется:
#   1. контроллер EHCI инициализируется, порт видит high-speed устройство;
#   2. накопитель перечисляется (control transfers) и виден в block layer;
#   3. BOT+SCSI через async schedule: монтирование и чтение файла;
#   4. hotplug: отсоединение убирает диск, подключение возвращает.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${EHCI_TEST_TIMEOUT:-180}"
ready_timeout="${BOOT_READY_TIMEOUT:-90}"

image="$log_dir/hdd.img"
usb_img="$log_dir/ehci-test.img"
serial_log="$log_dir/qemu-ehci.log"

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

echo "==> Preparing a FAT32 volume for the EHCI disk"
rm -f "$usb_img"
truncate -s 64M "$usb_img"
mkfs.fat -F 32 -n EHCITEST "$usb_img" >/dev/null
printf 'hello from ehci\n' > "$log_dir/ehci-hello.txt"
mcopy -i "$usb_img" "$log_dir/ehci-hello.txt" ::/HELLO.TXT

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

echo "==> Booting with an EHCI controller and a USB2 disk"
rm -f "$serial_log"
{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -Fq 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 3

    send_text "disks";                     send_text $'\n'; sleep 2
    send_text "mount /mnt auto sdb";       send_text $'\n'; sleep 3
    send_text "cat /mnt/hello.txt";        send_text $'\n'; sleep 2
    send_text "umount /mnt";               send_text $'\n'; sleep 2

    # Hotplug: выдернуть накопитель и вставить снова.
    echo "device_del usbdisk"; sleep 4
    send_text "disks";                     send_text $'\n'; sleep 2
    echo "device_add usb-storage,drive=usbfat2,bus=ehci.0,id=usbdisk2"
    sleep 5
    send_text "disks";                     send_text $'\n'; sleep 2
    send_text "mount /mnt auto sdb";       send_text $'\n'; sleep 3
    send_text "cat /mnt/hello.txt";        send_text $'\n'; sleep 2
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
    -device "usb-ehci,id=ehci" \
    -drive "if=none,id=usbfat,file=$usb_img,format=raw" \
    -device "usb-storage,drive=usbfat,bus=ehci.0,id=usbdisk" \
    -drive "if=none,id=usbfat2,file=$usb_img,format=raw" \
    || true

fail() {
    echo "ehci test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 160 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

grep -Fq '[EHCI]' "$serial_log" \
    || fail "the EHCI controller was not initialized"
grep -Fq 'connected, high speed' "$serial_log" \
    || fail "the port did not report a high-speed device"
grep -Fq ' -> storage' "$serial_log" \
    || fail "the disk was not enumerated as mass storage"
grep -Fq 'hello from ehci' "$serial_log" \
    || fail "the disk was not readable through the VFS"

# Hotplug: отключение и повторное подключение.
grep -Eq '\[EHCI\] hotplug: port [0-9]+ disconnected' "$serial_log" \
    || fail "hotplug did not notice the disk being removed"
grep -Fq '[USB] hotplug: block devices refreshed' "$serial_log" \
    || fail "block layer was not refreshed after hotplug"
grep -Eq '\[EHCI\] hotplug: port [0-9]+ connected' "$serial_log" \
    || fail "hotplug did not re-enumerate the disk"
[[ "$(grep -c 'hello from ehci' "$serial_log")" -ge 2 ]] \
    || fail "the disk was not readable after re-plugging"

if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic during the EHCI test"
fi
if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message during the EHCI test"
fi

echo "ehci test passed; serial log: $serial_log"