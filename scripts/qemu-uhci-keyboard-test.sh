#!/usr/bin/env bash
# qemu-uhci-keyboard-test.sh — проверка USB-клавиатуры на UHCI.
#
# На чипсетах Intel/AMD клавиатуры low-speed подключены к UHCI-компаньону,
# а не к xHCI. Тест поднимает UHCI-контроллер с usb-kbd и печатает команду
# с клавиатуры: если UHCI-драйвер не работает, shell останется пустым.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${UHCI_TEST_TIMEOUT:-150}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
log_file="$repo_root/build/qemu-uhci-keyboard.log"
report="$repo_root/build/uhci-keyboard-status.txt"
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

mkdir -p "$repo_root/build"
rm -f "$log_file"
: > "$report"

# Между клавишами нужна пауза: контроллер опрашивает порт раз в кадр (1 мс),
# а QEMU отправляет press+release почти мгновенно.
send_key() {
    echo "sendkey $1"
    sleep 0.3
}

{
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -Fq 'vibra:/>' "$log_file" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    sleep 2

    for key in e c h o spc u h c i; do send_key "$key"; done
    send_key ret
    sleep 2

    for key in q u i t; do send_key "$key"; done
    send_key ret
    sleep 3
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 256M -serial "file:$log_file" -monitor stdio -display none \
    -drive if=pflash,format=raw,file="$repo_root/OVMF.fd",readonly=on \
    -drive "file=$hdd",format=raw,snapshot=on -M q35 -no-reboot \
    -device piix3-usb-uhci,id=uhci \
    -device usb-kbd,bus=uhci.0 \
    >/dev/null 2>"$repo_root/build/qemu-uhci-stderr.log" || true

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

echo "==> Checking UHCI"
check "UHCI (I/O"
check "keyboard ready on port"
check "device(s) enumerated"
check "uhci"

if grep -Fq "KERNEL PANIC" "$log_file"; then
    echo "  [FAIL] kernel panic detected" | tee -a "$report" >&2
    failed=1
fi

if [[ "$failed" -ne 0 ]]; then
    echo "UHCI keyboard test failed; serial log: $log_file" >&2
    tail -60 "$log_file" >&2
    exit 1
fi

echo "UHCI keyboard test passed; serial log: $log_file"
