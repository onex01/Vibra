#!/usr/bin/env bash
# qemu-boot-log-test.sh — end-to-end проверка записи boot-лога на носитель.
#
# Что проверяется:
#   1. xHCI перечисляет USB mass storage (`usb-storage` в QEMU).
#   2. BOT/SCSI: INQUIRY, READ CAPACITY, READ(10), WRITE(10).
#   3. FAT32 RW: файл /VIBRA.LOG на разделе с меткой VIBRALOG.
#   4. Содержимое лога: начало загрузки, USB-хранилище и сам факт attach.
#
# Диск-приёмник — «superfloppy» FAT32 (без таблицы разделов): так его можно
# читать хостовыми mtools без вычисления offset'а раздела.
#
# Требования: qemu-system-x86_64, mtools, dosfstools.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${BOOT_LOG_TEST_TIMEOUT:-120}"
ready_timeout="${BOOT_READY_TIMEOUT:-60}"
# Маркер инкрементальной записи: печатается уже после загрузки, из shell.
# Только «нешафтовые» символы: `sendkey` в QEMU при повторных `shift-x`
# оставляет Shift зажатым, и верхний регистр/`-` ломают ввод.
marker="vibra-incremental-ok"

log_dir="$repo_root/build"
log_file="$log_dir/qemu-boot-log-test.log"
serial_ready="$log_dir/qemu-boot-log-serial.txt"
disk="$repo_root/build/boot-log-disk.img"
filler="$repo_root/build/boot-log-filler.bin"
content="$repo_root/build/boot-log-content.txt"
kernel_elf="$repo_root/target/x86_64-unknown-none/debug/vibra"
hdd="$repo_root/build/hdd.img"

for tool in "$qemu" mkfs.fat mcopy mtype dd; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done

if [[ ! -f "$repo_root/OVMF.fd" || ! -f "$hdd" ]]; then
    echo "missing boot artifacts; run ./build.sh setup && make build first" >&2
    exit 1
fi

echo "==> Building kernel with boot-log-to-usb"
(cd "$repo_root" && cargo +nightly build -p vibra-kernel --features boot-log-to-usb \
    -Z build-std=core,alloc,compiler_builtins -Z build-std-features)

echo "==> Installing kernel into hdd.img"
mcopy -o -i "$hdd" "$kernel_elf" ::/kernel.elf

echo "==> Preparing VIBRALOG disk (64 MiB, superfloppy FAT32)"
rm -f "$disk"
dd if=/dev/zero of="$disk" bs=1M count=64 status=none
mkfs.fat -F 32 -n VIBRALOG "$disk" >/dev/null
# Файл /VIBRA.LOG НЕ создаём: проверяем, что ядро само создаёт его
# (`Fat32Fs::create` + `alloc_cluster`) и публикует реальный размер.
mdir -i "$disk" ::/

qemu_key() {
    case "$1" in
        ' ') echo "spc" ;;
        $'\n') echo "ret" ;;
        '-') echo "minus" ;;
        '.') echo "dot" ;;
        '/') echo "slash" ;;
        # Верхний регистр требует shift: иначе sendkey молча теряет символ.
        [A-Z]) printf 'shift-%s\n' "$(printf '%s' "$1" | tr 'A-Z' 'a-z')" ;;
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

echo "==> Running QEMU with USB storage (${timeout_seconds}s)"
rm -f "$log_file" "$serial_ready"

{
    # Ждём приглашение shell: до него на диск ушла только «шапка» boot-лога.
    for _ in $(seq 1 "$ready_timeout"); do
        if grep -qa 'vibra:/>' "$log_file" 2>/dev/null; then
            break
        fi
        sleep 1
    done

    # Команда печатает в serial → кольцо → idle-цикл shell (раз в секунду)
    # дописывает хвост в /VIBRA.LOG. Так проверяется инкрементальная запись,
    # а не только разовый сброс накопленного лога при attach.
    send_text "echo $marker"
    send_text $'\n'
    sleep 3
    echo "quit"
} | timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 256M -serial "file:$log_file" -monitor stdio -display none \
    -drive if=pflash,format=raw,file="$repo_root/OVMF.fd",readonly=on \
    -drive file="$hdd",format=raw -M q35 -no-reboot \
    -device nec-usb-xhci,id=xhci \
    -drive if=none,id=logdisk,file="$disk",format=raw \
    -device usb-storage,drive=logdisk,bus=xhci.0 \
    >/dev/null 2>"$log_dir/qemu-boot-log-stderr.log" || true

if [[ -s "$log_dir/qemu-boot-log-stderr.log" ]]; then
    echo "  qemu stderr:"
    sed 's/^/    /' "$log_dir/qemu-boot-log-stderr.log"
fi

echo "==> Reading back /VIBRA.LOG"
mtype -i "$disk" ::/VIBRA.LOG > "$content" 2>/dev/null || true

fail=0
check() {
    local pattern="$1" description="$2"
    if grep -q -- "$pattern" "$content"; then
        echo "  ok: $description"
    else
        echo "  FAIL: $description (no '$pattern' in $content)" >&2
        fail=1
    fi
}

check '\[BOOT\] Step 1' 'boot log starts at Step 1'
check '\[xHCI\].*mass storage ready' 'xHCI configured bulk endpoints'
check '\[USB-STORAGE\].*sectors' 'SCSI READ CAPACITY answered'
check '\[LOG\] writing boot log to /VIBRA.LOG' 'log sink attached'
check '\[BOOT\] Step 9' 'log covers filesystem init'
check "$marker" 'log grew after boot (incremental flush)'

# Ядро само создаёт файл: на образе он должен быть ненулевым.
if mdir -i "$disk" ::/VIBRA.LOG >/dev/null 2>&1; then
    echo "  ok: /VIBRA.LOG exists on the volume"
else
    echo "  FAIL: /VIBRA.LOG missing on the volume" >&2
    fail=1
fi

if command -v fsck.fat >/dev/null 2>&1; then
    if fsck.fat -n "$disk" >/dev/null 2>&1; then
        echo "  ok: FAT32 volume is consistent (fsck.fat)"
    else
        echo "  FAIL: fsck.fat reported errors" >&2
        fsck.fat -n "$disk" >&2 || true
        fail=1
    fi
fi

if [[ -f "$log_file" ]]; then
    cp "$log_file" "$serial_ready"
fi

if [[ "$fail" -ne 0 ]]; then
    echo >&2
    echo "--- log tail ---" >&2
    tail -40 "$content" >&2 || true
    exit 1
fi

echo "==> Boot log test passed ($(stat -c%s "$content") bytes written to /VIBRA.LOG)"
