#!/usr/bin/env bash
# qemu-nvme-test.sh — сквозная проверка NVMe и mount auto.
#
# NVMe подключается как второй диск (sdb) с FAT32-образом. Проверяется:
#   * Identify Controller/Namespace, I/O queues;
#   * регистрация `sdb` в block layer;
#   * `mount /mnt auto sdb` и чтение файла.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
log_dir="$repo_root/build"
image="$log_dir/hdd.img"
nvme_img="$log_dir/nvme-test.img"
serial_log="$log_dir/qemu-nvme-test.log"
timeout_seconds="${NVME_TEST_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
QEMU="${QEMU:-qemu-system-x86_64}"

for tool in "$QEMU" mkfs.fat mcopy; do
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

echo "==> Preparing NVMe image (64 MiB FAT32)"
rm -f "$nvme_img"
dd if=/dev/zero of="$nvme_img" bs=1M count=64 status=none
mkfs.fat -F 32 -n NVMEVOL "$nvme_img" >/dev/null
printf 'hello nvme\n' > "$log_dir/nvme-hello.txt"
mcopy -i "$nvme_img" "$log_dir/nvme-hello.txt" ::/HELLO.TXT

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
    sleep 3
    send_text "mount /mnt auto sdb"; send_text $'\n'; sleep 4
    send_text "ls /mnt"; send_text $'\n'; sleep 2
    send_text "cat /mnt/hello.txt"; send_text $'\n'; sleep 2
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" \
    "$QEMU" \
    -m 512M \
    -serial "file:$serial_log" \
    -monitor stdio \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$image,format=raw" \
    -M q35 \
    -display none \
    -no-reboot \
    -drive "if=none,id=nvmedisk,file=$nvme_img,format=raw" \
    -device "nvme,drive=nvmedisk,serial=TESTNVME" || true

fail() {
    echo "nvme test failed: $1" >&2
    [[ ! -f "$serial_log" ]] || tail -n 160 "$serial_log" >&2
    exit 1
}
[[ -f "$serial_log" ]] || fail "no serial log"

grep -Fq 'I/O queues created' "$serial_log" || fail "NVMe I/O queues were not created"
grep -Fq 'Namespace 1:' "$serial_log" || fail "NVMe namespace was not identified"
grep -Fq '[NVMe, ' "$serial_log" || fail "NVMe disk was not registered in block layer"
grep -Fq 'OK (fat32, ro)' "$serial_log" || fail "mount auto did not mount the NVMe FAT32 volume"
grep -Fq 'hello nvme' "$serial_log" || fail "file on NVMe volume could not be read"
grep -Fq 'HELLO.TXT' "$serial_log" || fail "ls /mnt did not show the file"

if grep -Fq '[FATAL]' "$serial_log"; then
    fail "fatal message in NVMe boot log"
fi
if grep -Fq 'KERNEL PANIC' "$serial_log"; then
    fail "kernel panic on NVMe"
fi

echo "nvme test passed; serial log: $serial_log"