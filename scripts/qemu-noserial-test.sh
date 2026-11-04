#!/usr/bin/env bash
# qemu-noserial-test.sh — загрузка на машине без COM1.
#
# Регрессия: `serial::poll_key()` доверял `LSR.DR` и не проверял наличие
# UART. Без COM1 порт 0x3F8 не декодируется и читается как 0xFF, то есть
# `DATA_READY` (бит 0) всегда 1 — `try_read_byte()` бесконечно возвращал
# 0xFF, `decode_serial_byte(0xFF)` возвращал None, и цикл в `poll_key()`
# никогда не заканчивался. Первый вызов идёт из `heap_stress()` →
# `poll_input()`, поэтому ядро вставало ровно на строке
#
#   [BOOT] Step 4: Heap stress
#
# (ровно это видно на ноутбуке без физического COM-порта).
#
# Проверка не может читать serial-лог (его нет), поэтому ядро пишет маркеры
# на debugcon (порт 0xE9): `-debugcon file:` создаёт независимый канал.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${NOSERIAL_TEST_TIMEOUT:-120}"

image="$log_dir/hdd.img"
serial_log="$log_dir/qemu-noserial.log"
debug_log="$log_dir/qemu-noserial-debugcon.log"

for file in "$image" "$repo_root/OVMF.fd"; do
    if [[ ! -f "$file" ]]; then
        echo "missing $file (run ./build.sh setup && make build)" >&2
        exit 1
    fi
done

echo "==> Booting QEMU with -serial none (no COM1 at all)"
rm -f "$serial_log" "$debug_log"

# -serial none: контроллер 16550 не подключается, порты 0x3F8.. читаются
# как 0xFF — как на плате, где UART отключён в прошивке.
timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 512M \
    -serial none \
    -debugcon "file:$debug_log" \
    -global isa-debugcon.iobase=0xe9 \
    -monitor none \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw,snapshot=on" \
    -M q35 \
    -display none \
    -no-reboot || status=$?

fail() {
    echo "no-serial boot test failed: $1" >&2
    if [[ -f "$debug_log" ]]; then
        echo "--- debugcon (last 40 lines) ---" >&2
        tail -n 40 "$debug_log" >&2
    else
        echo "--- debugcon log was never created ---" >&2
    fi
    [[ ! -f "$serial_log" ]] || { echo "--- serial ---" >&2; tail -n 40 "$serial_log" >&2; }
    exit 1
}

[[ -f "$debug_log" ]] || fail "kernel produced no debugcon output at all"

grep -Fq '[BOOT] step 4: heap stress' "$debug_log" \
    || fail "kernel did not even reach the heap stress step"
# Без этой строки ядро зависло внутри heap_stress -> poll_input -> poll_key.
grep -Fq '[BOOT] step 4 done' "$debug_log" \
    || fail "boot hung in heap stress (poll_key busy-loops without a UART)"
grep -Fq '[BOOT] shell ready' "$debug_log" \
    || fail "kernel did not reach the shell without a UART"

echo "no-serial boot test passed; debugcon log: $debug_log"
