#!/usr/bin/env bash
# build-log-partition.sh — FAT32-образ с меткой VIBRALOG для `iso-debug`.
#
# Использование: build-log-partition.sh [size_mb] [output]
# Печатает путь к созданному образу.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
size_mb="${1:-${LOG_PARTITION_MB:-64}}"
output="${2:-$repo_root/build/log-partition.img}"

for tool in dd mkfs.fat mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required host command: $tool" >&2
        exit 127
    fi
done

if (( size_mb < 33 )); then
    echo "log partition must be at least 33 MiB (FAT32 minimum)" >&2
    exit 2
fi

mkdir -p "$(dirname "$output")"
rm -f "$output"

dd if=/dev/zero of="$output" bs=1M count="$size_mb" status=none
mkfs.fat -F 32 -n VIBRALOG "$output" >/dev/null

# Пустой /VIBRA.LOG: цепочку кластеров ядро выделит само по мере записи.
empty="$(mktemp)"
trap 'rm -f "$empty"' EXIT
: > "$empty"
mcopy -i "$output" "$empty" ::/VIBRA.LOG

echo "$output"
