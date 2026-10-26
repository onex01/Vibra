#!/usr/bin/env bash
# build-usb-debug.sh — загрузочная флешка с отдельным разделом для boot-лога.
#
# Раскладка (GPT, UEFI):
#   1. VIBRA-ESP  — FAT32, 64 МиБ: Limine + kernel.elf + limine.conf
#   2. VIBRALOG   — FAT32, остальное: пустой /VIBRA.LOG (ядро допишет)
#
# Ядро ищет раздел по метке тома (BPB 0x47), а не по номеру: метка VIBRALOG
# и есть второе «согласие» на запись (первое — feature boot-log-to-usb).
#
# BIOS-загрузка для GPT-образа не поддерживается (нужен отдельный BIOS boot
# раздел); ThinkPad T450 грузится в UEFI-режиме.
#
# Запись на флешку: sudo dd if=build/vibra-usb-debug.img of=/dev/sdX bs=4M status=progress
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$repo_root/build"
size_mb="${USB_DEBUG_SIZE_MB:-256}"
esp_mb="${USB_DEBUG_ESP_MB:-64}"
image="${USB_DEBUG_IMAGE:-$build_dir/vibra-usb-debug.img}"
limine_dir="${LIMINE_DIR:-$repo_root/limine-bin}"
kernel="${KERNEL_ELF:-$repo_root/target/x86_64-unknown-none/debug/vibra}"

esp_start=2048

if [[ "$image" != /* ]]; then
    image="$repo_root/$image"
fi

for tool in truncate sfdisk dd mkfs.fat mmd mcopy mdir; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required host command: $tool" >&2
        exit 127
    fi
done

for file in "$kernel" "$repo_root/limine.conf" "$limine_dir/BOOTX64.EFI"; do
    if [[ ! -f "$file" ]]; then
        echo "missing required file: $file (run make kernel && ./build.sh setup first)" >&2
        exit 1
    fi
done

if (( esp_mb < 33 )); then
    echo "USB_DEBUG_ESP_MB must be >= 33 (FAT32 minimum)" >&2
    exit 2
fi
if (( size_mb < esp_mb + 64 )); then
    echo "USB_DEBUG_SIZE_MB must be at least ESP + 64 MiB" >&2
    exit 2
fi

mkdir -p "$build_dir" "$(dirname "$image")"
rm -f "$image"

esp_sectors=$((esp_mb * 2048))
log_start=$((esp_start + esp_sectors))
log_sectors=$((size_mb * 2048 - log_start - 34))

esp_image="$(mktemp "$build_dir/usb-debug-esp.XXXXXX.img")"
log_image="$(mktemp "$build_dir/usb-debug-log.XXXXXX.img")"
trap 'rm -f "$esp_image" "$log_image"' EXIT

echo "==> Creating GPT image ($size_mb MiB: ESP ${esp_mb} MiB + VIBRALOG)"
truncate -s "${size_mb}M" "$image"
printf 'label: gpt\nunit: sectors\nfirst-lba: %s\n\nstart=%s, size=%s, type=U, name="VIBRA-ESP"\nstart=%s, size=%s, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name="VIBRALOG"\n' \
    "$esp_start" "$esp_start" "$esp_sectors" "$log_start" "$log_sectors" \
    | sfdisk --wipe always "$image" >/dev/null

echo "==> Building EFI System Partition"
dd if=/dev/zero of="$esp_image" bs=512 count="$esp_sectors" status=none
mkfs.fat -F 32 -n VIBRA-ESP "$esp_image" >/dev/null
mmd -i "$esp_image" ::/EFI ::/EFI/BOOT ::/limine
mcopy -o -i "$esp_image" "$limine_dir/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -o -i "$esp_image" "$kernel" ::/kernel.elf
mcopy -o -i "$esp_image" "$repo_root/limine.conf" ::/limine/limine.conf
dd if="$esp_image" of="$image" bs=512 seek="$esp_start" conv=notrunc status=none

echo "==> Building VIBRALOG partition"
dd if=/dev/zero of="$log_image" bs=512 count="$log_sectors" status=none
mkfs.fat -F 32 -n VIBRALOG "$log_image" >/dev/null
# Пустой файл: ядро дописывает его и публикует реальный размер.
: > "$build_dir/empty.log"
mcopy -i "$log_image" "$build_dir/empty.log" ::/VIBRA.LOG
rm -f "$build_dir/empty.log"
dd if="$log_image" of="$image" bs=512 seek="$log_start" conv=notrunc status=none

sfdisk --verify "$image"
mdir -i "$image@@$((esp_start * 512))" ::/limine >/dev/null
mdir -i "$image@@$((log_start * 512))" ::/ | sed 's/^/    /'

echo
echo "USB debug image ready: $image"
echo "  ESP:     offset $((esp_start * 512)) bytes, label VIBRA-ESP"
echo "  VIBRALOG: offset $((log_start * 512)) bytes, label VIBRALOG"
echo "Flash: sudo dd if=$image of=/dev/sdX bs=4M status=progress"
echo "Read the log afterwards: scripts/read-boot-log.sh /dev/sdX"
echo "No physical device was modified."
