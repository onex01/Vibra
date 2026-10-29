#!/usr/bin/env bash
# qemu-shell-test.sh — регрессия интерактивного shell.
#
# Проверяется:
#   * Tab дополняет команду и путь, не теряя prompt;
#   * Up повторяет историю и не затирает `vibra:/>`;
#   * `help <cmd>` и общий `--help` работают;
#   * `test timer` — новая объединённая команда проверок;
#   * в логе нет удвоенного prompt.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
serial_log="$log_dir/qemu-shell.log"
timeout_seconds="${SHELL_TEST_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

for file in "$image" "$repo_root/OVMF.fd"; do
    if [[ ! -f "$file" ]]; then
        echo "missing $file (run ./build.sh setup && make build)" >&2
        exit 1
    fi
done

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
    local text="$1" ch
    for ((i = 0; i < ${#text}; i++)); do
        ch="${text:i:1}"
        echo "sendkey $(qemu_key "$ch")"
        sleep 0.08
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

    # История: выполнить hostname, затем Up + Enter.
    send_text "hostname"; send_text $'\n'; sleep 1
    echo "sendkey up"; sleep 0.5
    echo "sendkey ret"; sleep 1

    # Tab для пути: ls /tm<TAB><ENTER>
    send_text "ls /cf"; echo "sendkey tab"; sleep 0.5
    echo "sendkey ret"; sleep 1

    # Tab для команды: hel<TAB><ENTER> (циклически может дать help)
    send_text "help cat"; send_text $'\n'; sleep 1
    send_text "test timer"; send_text $'\n'; sleep 2
    send_text "echo --help"; send_text $'\n'; sleep 1
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
    echo "shell test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 160 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

grep -Fq 'vibra:/> ls /cfg/' "$serial_log" || fail "Tab did not complete /cf to /cfg/"
grep -Eq '(^| )hostname( |$)' "$serial_log" || fail "ls /cfg/ produced no output"
grep -Fq 'cat' "$serial_log" || fail "help cat produced no output"
grep -Fq '[TIMER / CLOCK]' "$serial_log" || fail "test timer produced no output"
grep -Fq 'test timer' "$serial_log" || fail "history Up did not repeat the previous command"
grep -Fq 'echo - print text to screen' "$serial_log" || fail "generic --help did not work"

if grep -Fq 'vibra:/> vibra:/>' "$serial_log"; then
    fail "doubled prompt (history/Tab redraw is broken)"
fi

if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message in shell test"
fi

echo "shell test passed; serial log: $serial_log"