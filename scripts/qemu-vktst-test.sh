#!/usr/bin/env bash
# qemu-vktst-test.sh — проверка vktst, top, free и tree через serial.
#
# Что проверяется:
#   * vktst run all: группы cpu/mem/timer/fs/disk/net/kernel/log;
#   * free: память в GiB/MiB и проценты;
#   * top -b: CPU, память, диск, сеть и таблица процессов;
#   * tree из /proc показывает /proc, а не корень /.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${VKTST_TEST_TIMEOUT:-150}"
ready_timeout="${BOOT_READY_TIMEOUT:-45}"
log_file="$repo_root/build/qemu-vktst-test.log"
report="$repo_root/build/vktst-test-status.txt"
hdd="$repo_root/build/hdd.img"

for tool in "$qemu"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done

if [[ ! -f "$repo_root/OVMF.fd" || ! -f "$hdd" ]]; then
    echo "missing boot artifacts; run ./build.sh setup && make build first" >&2
    exit 1
fi

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
    make -C "$repo_root" build >/dev/null
fi

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
        sleep 0.05
    done
}

mkdir -p "$repo_root/build"
# Старый лог содержит prompt прошлой загрузки: без удаления readiness-loop
# срабатывает сразу и первые команды уходят в ещё не загруженное ядро.
rm -f "$log_file"
: > "$report"

{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -Fq 'vibra:/>' "$log_file" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 1

    send_text "vktst run all"; send_text $'\n'; sleep 12
    send_text "free";           send_text $'\n'; sleep 2
    send_text "top -b";         send_text $'\n'; sleep 4
    send_text "cd /proc";       send_text $'\n'; sleep 1
    send_text "tree";           send_text $'\n'; sleep 2
    send_text "quit";           send_text $'\n'; sleep 2
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 256M -serial "file:$log_file" -monitor stdio -display none \
    -drive if=pflash,format=raw,file="$repo_root/OVMF.fd",readonly=on \
    -drive "file=$hdd",format=raw -M q35 -no-reboot \
    -device e1000,netdev=net0 -netdev user,id=net0 \
    >/dev/null 2>"$repo_root/build/qemu-vktst-stderr.log" || true

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

echo "==> Checking vktst"
for group in cpu mem timer fs disk net kernel log; do
    check "[ OK ] $group"
done
check "0 failed"

echo "==> Checking free/top/tree"
check "Mem:"
check "Heap:"
check "Vibra top"
check "CPU  ["
check "kshell"
check "/proc"

if grep -Fq "KERNEL PANIC" "$log_file"; then
    echo "  [FAIL] kernel panic detected" | tee -a "$report" >&2
    failed=1
fi

if [[ "$failed" -ne 0 ]]; then
    echo "vktst test failed; serial log: $log_file" >&2
    tail -100 "$log_file" >&2
    exit 1
fi

echo "vktst test passed; serial log: $log_file"
