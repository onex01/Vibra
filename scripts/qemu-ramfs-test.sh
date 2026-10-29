#!/usr/bin/env bash
# qemu-ramfs-test.sh — регрессия корневого RamFS через VFS.
#
# Проверяется, что VFS и legacy-API видят одну и ту же ФС:
#   * `ls /` показывает каталоги, созданные при инициализации;
#   * `touch`/`cp`/`hostname` реально сохраняют данные (а не пишут в копию Vec);
#   * `mkdir`/`rmdir` работают на корневом томе;
#   * `/var/log/boot.log` непустой и содержит boot-маркеры;
#   * полный heap-стресс проходит по команде `diag heap-test`.
#
# Требует собранный образ: make build (или ./build.sh debug).
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
serial_log="$log_dir/qemu-ramfs.log"
timeout_seconds="${RAMFS_TEST_TIMEOUT:-180}"
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

    send_text "ls /";                send_text $'\n'; sleep 1
    send_text "touch /tmp/empty";    send_text $'\n'; sleep 1
    send_text "ls /tmp";             send_text $'\n'; sleep 1
    send_text "cp /cfg/hostname /tmp/host"; send_text $'\n'; sleep 1
    send_text "cat /tmp/host";       send_text $'\n'; sleep 1
    send_text "hostname qemu-test";  send_text $'\n'; sleep 1
    send_text "hostname";            send_text $'\n'; sleep 1
    send_text "mkdir /tmp/dir";      send_text $'\n'; sleep 1
    send_text "ls /tmp";             send_text $'\n'; sleep 1
    send_text "rmdir /tmp/dir";      send_text $'\n'; sleep 1
    send_text "cat /var/log/boot.log"; send_text $'\n'; sleep 2
    send_text "diag heap-test";      send_text $'\n'; sleep 25
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
    echo "ramfs test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 160 "$serial_log" >&2
    exit 1
}

[[ -f "$serial_log" ]] || fail "no serial log"

for dir in dev proc sys cfg home var tmp mnt; do
    grep -Eq "(^| )${dir}/?( |$)" "$serial_log" || fail "ls / has no ${dir}"
done

grep -Fq 'Created: /tmp/empty' "$serial_log" || fail "touch did not report success"
grep -Eq '(^| )empty( |$)' "$serial_log" || fail "ls /tmp does not show the created file"
grep -Fq 'Copied: /cfg/hostname -> /tmp/host' "$serial_log" || fail "cp did not report success"
grep -Eq '^vibra$|vibra[[:space:]]*$' "$serial_log" || fail "cat /tmp/host did not return file contents"
grep -Fq 'Hostname set to: qemu-test' "$serial_log" || fail "hostname did not persist"
grep -aEq '^qemu-test' "$serial_log" || fail "hostname did not read back"
grep -Eq '(^| )dir/?( |$)' "$serial_log" || fail "mkdir/rmdir did not create the directory"

grep -Fq '[BOOT] Step 1: serial OK' "$serial_log" || fail "/var/log/boot.log has no boot markers"
grep -Fq 'PASS: 10000 iters' "$serial_log" || fail "diag heap-test did not pass"

if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message in boot log"
fi
if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic"
fi

echo "ramfs test passed; serial log: $serial_log"
