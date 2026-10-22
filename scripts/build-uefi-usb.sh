#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$repo_root/build"
size_mb="${USB_SIZE_MB:-128}"
image="${UEFI_USB_IMAGE:-$build_dir/vibra-uefi.img}"
limine_dir="${LIMINE_DIR:-$repo_root/limine-bin}"
kernel="$repo_root/target/x86_64-unknown-none/debug/vibra"
partition_start=2048

if [[ "$image" != /* ]]; then
    image="$repo_root/$image"
fi
if ! [[ "$size_mb" =~ ^[0-9]+$ ]] || (( size_mb < 64 )); then
    echo "USB_SIZE_MB must be an integer >= 64" >&2
    exit 2
fi

host_has() {
    command -v "$1" >/dev/null 2>&1 && return 0
    command -v flatpak-spawn >/dev/null 2>&1 && flatpak-spawn --host sh -c "command -v $1 >/dev/null 2>&1"
}

host_run() {
    if command -v "$1" >/dev/null 2>&1; then
        "$@"
    elif command -v flatpak-spawn >/dev/null 2>&1; then
        flatpak-spawn --host "$@"
    else
        echo "missing host command: $1" >&2
        return 127
    fi
}

for tool in truncate sfdisk dd mkfs.fat mmd mcopy mdir; do
    if ! host_has "$tool"; then
        echo "missing required host command: $tool" >&2
        exit 127
    fi
done

for file in "$kernel" "$repo_root/limine.conf" "$limine_dir/BOOTX64.EFI"; do
    if [[ ! -f "$file" ]]; then
        echo "missing required file: $file (build the kernel and run setup first)" >&2
        exit 1
    fi
done

if [[ -e "$image" && "${UEFI_USB_OVERWRITE:-0}" != "1" ]]; then
    echo "refusing to overwrite existing image: $image" >&2
    echo "choose UEFI_USB_IMAGE=... or set UEFI_USB_OVERWRITE=1" >&2
    exit 1
fi

mkdir -p "$(dirname "$image")" "$build_dir"
if [[ "${UEFI_USB_OVERWRITE:-0}" == "1" ]]; then
    rm -f "$image"
fi

partition_sectors=$((size_mb * 2048 - partition_start - 33))
partition_image="$(mktemp "$build_dir/uefi-esp.XXXXXX.img")"
trap 'rm -f "$partition_image"' EXIT

host_run truncate -s "${size_mb}M" "$image"
printf 'label: gpt\nunit: sectors\nfirst-lba: %s\n\nstart=%s, size=%s, type=U, name="VIBRA-ESP"\n' \
    "$partition_start" "$partition_start" "$partition_sectors" \
    | host_run sfdisk --wipe always "$image"

host_run dd if=/dev/zero of="$partition_image" bs=512 count="$partition_sectors" status=none
host_run mkfs.fat -F 32 -n VIBRA "$partition_image"
host_run mmd -i "$partition_image" ::/EFI ::/EFI/BOOT ::/limine
host_run mcopy -o -i "$partition_image" "$limine_dir/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
host_run mcopy -o -i "$partition_image" "$kernel" ::/kernel.elf
host_run mcopy -o -i "$partition_image" "$repo_root/limine.conf" ::/limine/limine.conf
host_run dd if="$partition_image" of="$image" bs=512 seek="$partition_start" conv=notrunc status=none

host_run sfdisk --verify "$image"
host_run mdir -i "$image@@$((partition_start * 512))" ::/EFI/BOOT
host_run mdir -i "$image@@$((partition_start * 512))" ::/limine
echo "UEFI test image ready: $image (${size_mb} MiB, GPT EFI System Partition)"
echo "Secure Boot is not supported by the bundled Limine binary; disable it for this test."
echo "No physical device was modified."