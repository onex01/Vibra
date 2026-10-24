#!/usr/bin/env bash
# Проверяет часы ядра и RTC на живой загрузке:
#   * `date` печатает реальную дату (не uptime);
#   * `uptime` использует монотонные часы и показывает источник;
#   * `diag timer` показывает TSC/HPET, частоту IRQ-тиков и RTC;
#   * `/proc/uptime` отдаёт монотонные секунды;
#   * корень рамфс содержит только реальные каталоги (нет /os, /apps, /lib).
#
# Требует собранный образ: make build (или ./build.sh debug).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
serial_log="$log_dir/qemu-clock.log"
timeout_seconds="${CLOCK_TEST_TIMEOUT:-90}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

if [[ ! -f "$image" ]]; then
    echo "missing $image; run 'make build' first" >&2
    exit 1
fi
if [[ ! -f "$repo_root/OVMF.fd" ]]; then
    echo "missing OVMF.fd; run ./build.sh setup" >&2
    exit 1
fi

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '/') echo "slash" ;;
        '.') echo "dot" ;;
        '-') echo "minus" ;;
        *) echo "$1" ;;
    esac
}

send_text() {
    local text="$1"
    local ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.04
    done
}

rm -f "$serial_log"

{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -q 'vibra:/>' "$serial_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done

    send_text "date";            send_text $'\n'; sleep 1
    send_text "uptime";          send_text $'\n'; sleep 1
    send_text "diag timer";      send_text $'\n'; sleep 2
    send_text "cat /proc/uptime"; send_text $'\n'; sleep 1
    send_text "ls /";            send_text $'\n'; sleep 1
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
    "$QEMU" \
    -m 256M \
    -serial "file:$serial_log" \
    -monitor stdio \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw" \
    -M q35 \
    -display none \
    -no-reboot || true

fail() {
    echo "clock test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 120 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

# date: YYYY-MM-DD HH:MM:SS
grep -Eq '[12][0-9]{3}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}' "$serial_log" \
    || fail "date did not print a real date/time"

grep -Fq 'Clock:' "$serial_log" || fail "uptime did not print the clock source"

grep -Fq '[TIMER / CLOCK]' "$serial_log" || fail "diag timer produced no output"
grep -Fq 'source:' "$serial_log" || fail "diag timer did not print the clock source"
grep -Fq 'RTC:' "$serial_log" || fail "diag timer did not print RTC state"
grep -Eq 'IRQ ticks: *[0-9]+ total' "$serial_log" \
    || fail "diag timer did not print IRQ tick statistics"

# /proc/uptime: "<seconds>.<centiseconds> <idle>"
grep -Eq '^[0-9]+\.[0-9]{2} [0-9]+\.[0-9]{2}' "$serial_log" \
    || fail "/proc/uptime has no numeric value"

# Корень рамфс: реальные каталоги есть, отладочных дублей нет.
for dir in dev proc sys cfg home var tmp mnt; do
    grep -Eq "(^| )${dir}/?( |$)" "$serial_log" || fail "ls / has no ${dir}"
done
for dir in os apps lib; do
    if grep -Eq "(^| )${dir}/?( |$)" "$serial_log"; then
        fail "ls / still contains the removed directory /${dir}"
    fi
done

if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message in boot log"
fi

echo "clock test passed; serial log: $serial_log"
