#!/usr/bin/env bash
# read-boot-log.sh — прочитать /VIBRA.LOG с флешки/образа без монтирования.
#
# Использование:
#   scripts/read-boot-log.sh /dev/sdX            # вывод в stdout
#   scripts/read-boot-log.sh build/usb.img log.txt
#
# Работает через mtools: раздел ищется по метке тома VIBRALOG (superfloppy,
# MBR и GPT). Это безопасно: носитель не монтируется, ничего не пишется.
set -euo pipefail

target="${1:-}"
output="${2:-}"
if [[ -z "$target" ]]; then
    echo "usage: $0 <image-or-device> [output-file]" >&2
    exit 2
fi
if ! command -v mtype >/dev/null 2>&1; then
    echo "missing required host command: mtype (install mtools)" >&2
    exit 127
fi

work="$(mktemp)"
trap 'rm -f "$work"' EXIT

# mtype дополняет файл нулями до размера, поэтому NUL-байты вырезаем.
# Пустой /VIBRA.LOG — это валидный результат (лога ещё нет), поэтому
# ориентируемся на код возврата mtype, а не на размер: иначе только что
# созданный образ выглядел бы как «раздела нет».
extract() {
    local spec="$1"
    mtype -i "$spec" ::/VIBRA.LOG 2>/dev/null | tr -d '\000' > "$work"
}

found=""
# 1. «Superfloppy»: FAT32 прямо с сектора 0.
if extract "$target"; then
    found="whole disk"
fi

# 2. Разделы (MBR/GPT) через sfdisk, если он есть.
if [[ -z "$found" ]] && command -v sfdisk >/dev/null 2>&1; then
    while read -r start sectors; do
        [[ -z "${start:-}" || -z "${sectors:-}" ]] && continue
        if extract "$target@@$((start * 512))"; then
            found="partition at LBA $start"
            break
        fi
    done < <(sfdisk -d "$target" 2>/dev/null | sed -n 's/.*start=[[:space:]]*\([0-9]*\), size=[[:space:]]*\([0-9]*\).*/\1 \2/p')
fi

if [[ -z "$found" ]]; then
    echo "no VIBRALOG volume with /VIBRA.LOG found on $target" >&2
    exit 1
fi

size="$(stat -c%s "$work")"
if [[ "$size" -eq 0 ]]; then
    echo "note: /VIBRA.LOG on $target ($found) is empty — ядро ещё не писало лог" >&2
fi

if [[ -n "$output" ]]; then
    cp "$work" "$output"
    echo "boot log ($size bytes, $found) -> $output" >&2
else
    cat "$work"
fi
