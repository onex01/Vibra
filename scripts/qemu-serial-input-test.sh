#!/usr/bin/env bash
# qemu-serial-input-test.sh — ввод через COM1 (а не через monitor `sendkey`).
#
# Зачем отдельный тест: остальные shell-тесты подают команды через QEMU
# monitor, поэтому путь «serial IRQ4 -> ring buffer -> poll_key -> строка
# shell» не проверялся вообще. Именно в нём был баг: `try_read_byte()`
# доверял `LSR.DR` без проверки, есть ли UART, и на машине без COM1
# `poll_key()` зависал навсегда (ядро вставало на `[BOOT] Step 4`).
#
# Проверяется:
#   * `COM1: present` — loopback-проба порта;
#   * команда, набранная в serial, доходит до shell и выполняется;
#   * вторая команда выполняется после первой (очередь ввода не залипает).
#
# Требования: qemu-system-x86_64.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${SERIAL_INPUT_TEST_TIMEOUT:-120}"
serial_log="$log_dir/qemu-serial-input.log"
image="$log_dir/hdd.img"

if ! command -v "$qemu" >/dev/null 2>&1; then
    echo "missing QEMU executable: $qemu" >&2
    exit 127
fi
for file in "$image" "$repo_root/OVMF.fd"; do
    if [[ ! -f "$file" ]]; then
        echo "missing $file (run ./build.sh setup && make build)" >&2
        exit 1
    fi
done

echo "==> Booting QEMU with -serial stdio and typing into COM1"
rm -f "$serial_log"

{
    # Ждём, пока shell поднимется: serial-лог пишется в тот же поток.
    sleep 25
    printf 'echo serial-input-ok\r'
    sleep 3
    printf 'version\r'
    sleep 3
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 512M \
    -serial stdio \
    -monitor none \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw,snapshot=on" \
    -M q35 \
    -display none \
    -no-reboot >"$serial_log" 2>&1 || true

fail() {
    echo "serial input test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 80 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

grep -Fq 'COM1: present' "$serial_log" || fail "COM1 was not detected (loopback probe failed)"
grep -Fq 'serial-input-ok' "$serial_log" || fail "a command typed into COM1 was not executed"
# Вторая команда доказывает, что ввод не залип после первой.
grep -Fq 'Vibra 0' "$serial_log" || fail "the shell stopped reading input after the first command"

# Ядро должно дойти до shell, а не зависнуть на загрузке.
if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic while reading serial input"
fi

echo "serial input test passed; serial log: $serial_log"
