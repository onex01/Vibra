#!/usr/bin/env bash
# Проверяет ввод с клавиатуры в QEMU через monitor `sendkey`:
#   1) PS/2 (i8042) — polling fallback и IRQ1;
#   2) USB HID boot keyboard поверх xHCI.
#
# Требует собранный образ: ./build.sh debug (или make build).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
timeout_seconds="${KEYBOARD_TEST_TIMEOUT:-90}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"

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
        '-') echo "minus" ;;
        '.') echo "dot" ;;
        '/') echo "slash" ;;
        *) echo "$1" ;;
    esac
}

# Отправляет строку в QEMU monitor (по одному sendkey на символ).
send_text() {
    local text="$1"
    local ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.04
    done
}

run_case() {
    local name="$1"
    shift
    local serial_log="$log_dir/keyboard-$name.log"
    local qemu_args=("$@")

    echo "== keyboard case: $name =="
    rm -f "$serial_log"

    {
        # Ждём приглашение shell в serial-логе (boot занимает ~10-30 c).
        for _ in $(seq 1 "$ready_timeout"); do
            if grep -q 'vibra:/>' "$serial_log" 2>/dev/null; then
                break
            fi
            sleep 1
        done

        send_text "echo kbd-ok"
        send_text $'\n'
        sleep 1
        send_text "diag keyboard"
        send_text $'\n'
        sleep 3
        echo "quit"
    } | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
        qemu-system-x86_64 \
        -m 256M \
        -serial "file:$serial_log" \
        -monitor stdio \
        -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
        -drive "file=$image,format=raw" \
        -M q35 \
        -display none \
        -no-reboot \
        "${qemu_args[@]}" >/dev/null 2>&1 || true

    local failed=0
    if ! grep -q 'kbd-ok' "$serial_log"; then
        echo "  FAIL: команда, набранная с клавиатуры, не выполнилась"
        failed=1
    fi
    if [[ "$name" == "usb" ]] && ! grep -q 'HID keyboard ready' "$serial_log"; then
        echo "  FAIL: xHCI не перечислил USB HID клавиатуру"
        failed=1
    fi
    if [[ "$name" == "usb" ]] && ! grep -qE 'reports [1-9][0-9]*' "$serial_log"; then
        echo "  FAIL: HID-отчёты не дошли до драйвера"
        failed=1
    fi

    if [[ $failed -eq 0 ]]; then
        echo "  PASS ($serial_log)"
    else
        echo "  --- tail $serial_log ---"
        tail -40 "$serial_log" || true
        return 1
    fi
}

status=0
selection="${1:-all}"
if [[ "$selection" == "all" || "$selection" == "ps2" ]]; then
    run_case ps2 || status=1
fi
if [[ "$selection" == "all" || "$selection" == "usb" ]]; then
    run_case usb -device nec-usb-xhci,id=xhci -device usb-kbd,bus=xhci.0 || status=1
fi

exit $status
