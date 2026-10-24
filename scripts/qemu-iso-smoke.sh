#!/usr/bin/env bash
# Проверяет, что собранный ISO действительно загружается как съёмный носитель:
#   * UEFI — через OVMF и El Torito EFI-образ;
#   * BIOS — через SeaBIOS и El Torito limine-bios-cd.bin.
#
# ISO — это путь для реального железа, поэтому проверяется не только сборка,
# но и фактическая загрузка с `-cdrom` (в отличие от build/hdd.img).
#
# Требует: ./build.sh iso (или make iso).
# Использование: scripts/qemu-iso-smoke.sh [all|uefi|bios]
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
iso="${ISO_IMAGE:-$repo_root/build/vibra.iso}"
firmware="${OVMF:-$repo_root/OVMF.fd}"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${ISO_SMOKE_TIMEOUT:-90}"
selection="${1:-all}"
log_dir="$repo_root/build"

if ! command -v "$qemu" >/dev/null 2>&1; then
    echo "missing QEMU executable: $qemu" >&2
    exit 127
fi
if ! command -v timeout >/dev/null 2>&1; then
    echo "missing command: timeout" >&2
    exit 127
fi
if [[ ! -f "$iso" ]]; then
    echo "missing ISO image: $iso (run ./build.sh iso)" >&2
    exit 1
fi

run_case() {
    local name="$1"
    shift
    local log="$log_dir/iso-smoke-$name.log"
    local status=0

    echo "== ISO boot case: $name =="
    rm -f "$log"

    timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
        -m 512M \
        -serial "file:$log" \
        -monitor none \
        -cdrom "$iso" \
        -M q35 \
        -display none \
        -no-reboot \
        "$@" || status=$?

    # 124/137 — штатное завершение по timeout: ядро остаётся в shell.
    if [[ "$status" -ne 0 && "$status" -ne 124 && "$status" -ne 137 ]]; then
        echo "  FAIL: QEMU exited with status $status" >&2
        [[ ! -f "$log" ]] || tail -n 40 "$log" >&2
        return 1
    fi
    if [[ ! -f "$log" ]]; then
        echo "  FAIL: no serial log ($log)" >&2
        return 1
    fi

    local marker failed=0
    for marker in \
        "[BOOT] Step 1: serial OK" \
        "[TIMER] RTC (BIOS):" \
        "[VMM] Initialization complete!" \
        "vibra:/>"; do
        if ! grep -Fq "$marker" "$log"; then
            echo "  FAIL: missing marker: $marker"
            failed=1
        fi
    done
    if grep -Fq "[FATAL]" "$log"; then
        echo "  FAIL: fatal boot message found"
        failed=1
    fi

    if [[ "$failed" -eq 0 ]]; then
        echo "  PASS ($log)"
        return 0
    fi

    echo "  --- tail $log ---"
    tail -n 40 "$log" || true
    return 1
}

status=0
case "$selection" in
    all)
        if [[ -f "$firmware" ]]; then
            run_case uefi -drive "if=pflash,format=raw,file=$firmware,readonly=on" || status=1
        else
            echo "  SKIP uefi: missing $firmware (run ./build.sh setup)" >&2
            status=1
        fi
        run_case bios || status=1
        ;;
    uefi)
        if [[ ! -f "$firmware" ]]; then
            echo "missing UEFI firmware: $firmware (run ./build.sh setup)" >&2
            exit 1
        fi
        run_case uefi -drive "if=pflash,format=raw,file=$firmware,readonly=on" || status=1
        ;;
    bios)
        run_case bios || status=1
        ;;
    *)
        echo "Usage: $0 [all|uefi|bios]" >&2
        exit 2
        ;;
esac

exit "$status"
