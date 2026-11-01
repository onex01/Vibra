#!/usr/bin/env bash
# qemu-debug-image-test.sh — end-to-end проверка debug-образа с тремя разделами.
#
# Проверяется:
#   1. Образ загружается как диск (USB-сценарий), а не только как CD.
#   2. Ядро видит ESP, VIBRA (kernel) и VIBRALOG.
#   3. Boot-лог реально пишется в /VIBRA.LOG на VIBRALOG и читается хостом.
#
# Использование:
#   bash scripts/qemu-debug-image-test.sh <image>
#
# Для ISO тот же скрипт работает, если образ записан на USB: QEMU
# подключает его как обычный диск (`-drive file=...`), поэтому запись
# в VIBRALOG сохраняется.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${DEBUG_IMAGE_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
image="${1:-${DEBUG_IMAGE:-}}"

if [[ -z "$image" ]]; then
    echo "usage: $0 <debug image>" >&2
    exit 2
fi
if [[ "$image" != /* ]]; then
    image="$repo_root/$image"
fi
if [[ ! -f "$image" ]]; then
    echo "missing debug image: $image" >&2
    exit 1
fi

for tool in "$qemu" mtype sfdisk cp; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done

version="$(sed -n 's/^pub const KERNEL_VERSION: &str = "\(.*\)";/\1/p' "$repo_root/kernel/src/version.rs")"
log_file="$repo_root/build/qemu-debug-image.log"
test_image="$repo_root/build/debug-image-test.img"
report="$repo_root/build/debug-image-status.txt"

mkdir -p "$repo_root/build"
: > "$log_file"
: > "$report"

# Копия: тест пишет в образ, артефакт для релиза должен остаться чистым.
cp --sparse=always "$image" "$test_image"

# Ищем раздел по метке FAT (BPB 0x47), а не по имени GPT: у ISO
# xorriso называет appended-разделы Appended2/Appended3.
mapfile -t starts < <(sfdisk -d "$test_image" | sed -n 's/.*: start= *\([0-9][0-9]*\).*/\1/p')
start=""
for candidate in "${starts[@]}"; do
    label="$(dd if="$test_image" bs=512 skip="$candidate" count=1 status=none \
        | dd bs=1 skip=71 count=11 status=none 2>/dev/null || true)"
    if [[ "$label" == "VIBRALOG   " ]]; then
        start="$candidate"
        break
    fi
done
if [[ -z "$start" ]]; then
    echo "VIBRALOG partition not found in $image" >&2
    sfdisk -d "$test_image" | sed 's/^/  /' >&2
    exit 1
fi
offset_bytes=$((start * 512))

echo "==> Booting debug image (${timeout_seconds}s)"
timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 512M -serial "file:$log_file" -monitor none -display none \
    -drive if=pflash,format=raw,file="$repo_root/OVMF.fd",readonly=on \
    -drive "file=$test_image",format=raw -M q35 -no-reboot \
    >/dev/null 2>&1 || true

failed=0
check() {
    local marker="$1"
    if grep -Fq "$marker" "$log_file"; then
        echo "  [ OK ] $marker" | tee -a "$report"
    else
        echo "  [FAIL] $marker" | tee -a "$report" >&2
        failed=1
    fi
}

echo "==> Checking kernel view"
# ESP в ISO называется "EFI boot partition", а в USB-образе — VIBRA-ESP,
# поэтому проверяем только метку kernel-раздела и самого лога.
check "VIBRA"
check "/VIBRA.LOG"
check "writing logs to VIBRALOG"
check "vibra:/>"
check "Kernel v$version"

if grep -Fq "KERNEL PANIC" "$log_file"; then
    echo "  [FAIL] kernel panic detected" | tee -a "$report" >&2
    failed=1
fi

echo "==> Reading /VIBRA.LOG from the image"
if mtype -i "$test_image@@$offset_bytes" ::/VIBRA.LOG > "$repo_root/build/debug-image-boot.log" 2>/dev/null; then
    if grep -Fq "[BOOT]" "$repo_root/build/debug-image-boot.log"; then
        echo "  [ OK ] boot log persisted on VIBRALOG" | tee -a "$report"
    else
        echo "  [FAIL] boot log has no [BOOT] markers" | tee -a "$report" >&2
        failed=1
    fi
else
    echo "  [FAIL] cannot read /VIBRA.LOG from VIBRALOG" | tee -a "$report" >&2
    failed=1
fi

if [[ "$failed" -ne 0 ]]; then
    echo "Debug image test failed; serial log: $log_file" >&2
    tail -80 "$log_file" >&2
    exit 1
fi

echo "Debug image test passed; serial log: $log_file"
