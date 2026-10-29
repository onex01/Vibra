#!/usr/bin/env bash
# qemu-ro-mount-test.sh — проверка read-only по умолчанию.
#
# Сценарий:
#   1. FAT32-образ с HELLO.TXT подключается вторым диском (sdb).
#   2. `mount /mnt fat32 sdb` без `rw` — монтирование только для чтения.
#   3. `cat /mnt/hello.txt` работает, `cp` в том возвращает Read-only.
#   4. На хосте: COPY.TXT отсутствует, SHA256 образа не изменился.
#   5. Повторная загрузка с `mount /mnt fat32 sdb rw` — копия появляется,
#      fsck.fat не находит ошибок.
#
# Требует собранный hdd.img (`make build`) и OVMF.fd.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
ro_img="$log_dir/ro-mount-test.img"
serial_ro="$log_dir/qemu-ro-mount-ro.log"
serial_rw="$log_dir/qemu-ro-mount-rw.log"
hello="$log_dir/ro-mount-hello.txt"
timeout_seconds="${RO_MOUNT_TEST_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

for tool in "$QEMU" mkfs.fat mcopy mdir sha256sum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done
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
        sleep 0.04
    done
}

run_case() {
    local mode="$1" log="$2" mount_cmd="$3" cp_expected="$4"
    rm -f "$log"
    {
        for _ in $(seq 1 "$ready_timeout"); do
            if grep -q 'vibra:/>' "$log" 2>/dev/null; then
                break
            fi
            sleep 1
        done

        send_text "$mount_cmd";             send_text $'\n'; sleep 2
        send_text "cat /mnt/hello.txt";     send_text $'\n'; sleep 1
        send_text "cp /mnt/hello.txt /mnt/copy.txt"; send_text $'\n'; sleep 2
        send_text "ls /mnt";                send_text $'\n'; sleep 1
        echo "quit"
    } | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
        "$QEMU" \
        -m 256M \
        -serial "file:$log" \
        -monitor stdio \
        -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
        -drive "file=$image,format=raw" \
        -drive "file=$ro_img,format=raw" \
        -M q35 \
        -display none \
        -no-reboot || true

    [[ -f "$log" ]] || { echo "ro-mount test ($mode): no serial log" >&2; return 1; }
    grep -qa 'hello from ro mount' "$log" \
        || { echo "ro-mount test ($mode): cat did not read the file" >&2; tail -n 80 "$log" >&2; return 1; }
    grep -qa "$cp_expected" "$log" \
        || { echo "ro-mount test ($mode): expected '$cp_expected'" >&2; tail -n 80 "$log" >&2; return 1; }
}

echo "==> Preparing FAT32 test volume (64 MiB, label ROTEST)"
rm -f "$ro_img"
dd if=/dev/zero of="$ro_img" bs=1M count=64 status=none
mkfs.fat -F 32 -n ROTEST "$ro_img" >/dev/null
printf 'hello from ro mount\n' > "$hello"
mcopy -i "$ro_img" "$hello" ::/HELLO.TXT

before_hash="$(sha256sum "$ro_img" | awk '{print $1}')"

echo "==> Case 1: mount without rw (must be read-only)"
run_case ro "$serial_ro" "mount /mnt fat32 sdb" "Read-only file system"

after_hash="$(sha256sum "$ro_img" | awk '{print $1}')"
if [[ "$before_hash" != "$after_hash" ]]; then
    echo "ro-mount test failed: read-only mount modified the image" >&2
    exit 1
fi
if mdir -i "$ro_img" ::/COPY.TXT >/dev/null 2>&1; then
    echo "ro-mount test failed: COPY.TXT exists after read-only mount" >&2
    exit 1
fi
echo "  ok: image unchanged, no COPY.TXT"

echo "==> Case 2: mount with explicit rw"
run_case rw "$serial_rw" "mount /mnt fat32 sdb rw" "Copied: /mnt/hello.txt -> /mnt/copy.txt"

if ! mtype -i "$ro_img" ::/COPY.TXT 2>/dev/null | grep -qa 'hello from ro mount'; then
    echo "ro-mount test failed: COPY.TXT missing after rw mount" >&2
    exit 1
fi
echo "  ok: COPY.TXT written after explicit rw"

if command -v fsck.fat >/dev/null 2>&1; then
    if ! fsck.fat -n "$ro_img" >/dev/null 2>&1; then
        echo "ro-mount test failed: fsck.fat reported errors" >&2
        fsck.fat -n "$ro_img" >&2 || true
        exit 1
    fi
    echo "  ok: FAT32 volume is consistent"
fi

if grep -Fq '[FATAL]' "$serial_ro" "$serial_rw"; then
    echo "ro-mount test failed: fatal message in boot log" >&2
    exit 1
fi

echo "ro-mount test passed"
