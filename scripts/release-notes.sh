#!/usr/bin/env bash
# release-notes.sh — текст GitHub Release для текущей версии ядра.
#
# Собирает три вещи, чтобы описание релиза не надо было писать руками:
#   1. раздел из `CHANGELOG.md` под текущую версию (что изменилось);
#   2. список коммитов с предыдущего тега (проверяемость изменений);
#   3. таблицу образов из `dist/` с размерами и контрольными суммами.
#
# Использование:
#   scripts/release-notes.sh [--tag v0.2.0-rc.14] [--dist dist] [--out notes.md]
#
# Без `--tag` берётся `v$(версия из kernel/src/version.rs)`.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
version_file="$repo_root/kernel/src/version.rs"
changelog="$repo_root/CHANGELOG.md"

tag=""
dist_dir="$repo_root/dist"
out_file=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag) tag="${2:?}"; shift 2 ;;
        --dist) dist_dir="${2:?}"; shift 2 ;;
        --out) out_file="${2:?}"; shift 2 ;;
        -h|--help) sed -n '2,15p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

version="$(sed -n 's/^pub const KERNEL_VERSION: &str = "\(.*\)";/\1/p' "$version_file")"
codename="$(sed -n 's/^pub const KERNEL_CODENAME: &str = "\(.*\)";/\1/p' "$version_file")"
if [[ -z "$version" ]]; then
    echo "cannot read KERNEL_VERSION from $version_file" >&2
    exit 1
fi
[[ -n "$tag" ]] || tag="v$version"

# --- 1. Раздел CHANGELOG под эту версию --------------------------------------
# Заголовок вида `## 0.2.0-rc.14 «Foundation RC14» — 2026-11-04`, до
# следующего `## ` (или до `---`, которым разделены крупные блоки).
changelog_section="$(
    awk -v ver="$version" '
        $0 ~ "^## " && index($0, ver) > 0 { inside = 1; next }
        inside && /^## / { exit }
        inside && /^---[[:space:]]*$/ { exit }
        inside { print }
    ' "$changelog"
)"
if [[ -z "${changelog_section//[[:space:]]/}" ]]; then
    changelog_section="_В CHANGELOG.md нет раздела для версии $version._"
fi

# --- 2. Коммиты с предыдущего тега ------------------------------------------
# Тег текущего релиза может уже существовать (повторная публикация), поэтому
# ищем последний тег, который не равен нашему.
previous_tag=""
if git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
    while read -r candidate; do
        [[ -z "$candidate" || "$candidate" == "$tag" ]] && continue
        previous_tag="$candidate"
        break
    done < <(git -C "$repo_root" tag --sort=-v:refname | grep -E '^v' || true)
fi

if [[ -n "$previous_tag" ]]; then
    commit_range="$previous_tag..HEAD"
    commits_header="### Коммиты с \`$previous_tag\`"
else
    commit_range="HEAD"
    commits_header="### Коммиты"
fi

commits="$(
    git -C "$repo_root" log --no-merges --pretty='- %s (@%an)' "$commit_range" 2>/dev/null \
        | head -n 60 || true
)"
if [[ -z "$commits" ]]; then
    commits="_нет данных (сборка не из git-репозитория)_"
fi

diffstat=""
if [[ -n "$previous_tag" ]]; then
    diffstat="$(git -C "$repo_root" diff --shortstat "$previous_tag"..HEAD 2>/dev/null || true)"
fi

# --- 3. Таблица образов ------------------------------------------------------
artifacts=""
if [[ -d "$dist_dir" ]]; then
    artifacts="$(
        cd "$dist_dir"
        shopt -s nullglob
        for file in *.iso *.img *.zip *.xz; do
            [[ -f "$file" ]] || continue
            size="$(du -h --apparent-size "$file" | cut -f1)"
            hash=""
            if command -v sha256sum >/dev/null 2>&1; then
                hash="$(sha256sum "$file" | cut -c1-16)…"
            fi
            printf '| `%s` | %s | %s |\n' "$file" "$size" "${hash:-—}"
        done
    )"
fi
if [[ -z "$artifacts" ]]; then
    artifacts="| — | — | — |"
fi

# --- Сборка текста -----------------------------------------------------------
{
    cat <<EOF
**Версия:** \`$version\` «${codename:-Vibra}»
**Тег:** \`$tag\`
**Собрано из:** \`$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)\`

> Vibra — экспериментальное \`no_std\`-ядро на Rust для \`x86_64\`. Это не
> дистрибутив и не замена Linux: проект проверяет, что можно собрать
> работающее ядро с нуля и отладить его на реальном железе.

## Что изменилось

$(printf '%s\n' "$changelog_section" | sed '/^[[:space:]]*$/N;/^\n$/D')

EOF

    if [[ -n "$diffstat" ]]; then
        echo "$diffstat"
        echo
    fi

    cat <<EOF
## Образы

| Файл | Размер | SHA256 (начало) |
|------|--------|-----------------|
$artifacts

Что брать:

- **\`vibra-release_*.iso\`** — релизный ISO (UEFI+BIOS), обычная загрузка,
  UART выключен. **\`vibra_*.iso\`** — то же самое, но с включённым serial
  (debug-сборка без раздела логов).
- **\`vibra-debug_*.iso\`** — отладочный ISO: serial-лог, раздел \`VIBRALOG\`,
  \`/VIBRA.LOG\` и \`/SHELL.LOG\` на носителе.
- **\`vibra-usb_*.img\`** — образ для записи на флешку (\`dd\`), GPT+ESP.
- **\`vibra-usb-debug_*.img\`** — флешка с тремя разделами (ESP, VIBRA, VIBRALOG)
  для разбора проблем на реальной машине.
- **\`SHA256SUMS\`** — контрольные суммы всех файлов.

\`\`\`sh
sudo dd if=vibra-usb_*.img of=/dev/sdX bs=4M status=progress conv=fsync
\`\`\`

## $commits_header

$commits

## Ограничения

- Запись разрешена только на FAT32 и только с явным \`rw\`; ext2/3/4, NTFS,
  exFAT и ISO9660 — read-only.
- SMP не включён (используется одно ядро), хотя APIC/IOAPIC присутствуют.
- Поддержка железа проверяется на реальных машинах, а не по QEMU:
  см. \`docs/verified-hardware.md\`, список пробелов — \`docs/roadmap.md\`.

---
\`docs/commands.md\` — команды shell, \`docs/roadmap.md\` — план,
\`CHANGELOG.md\` — история, \`docs/testing.md\` — как прогнать тесты.
EOF
} > "${out_file:-/dev/stdout}"
