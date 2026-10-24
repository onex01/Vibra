#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
timeout_seconds="${SMOKE_TIMEOUT:-90}"
minimum_heartbeats="${SMOKE_MIN_HEARTBEATS:-12}"
log_file="$repo_root/build/qemu-smoke.log"
heartbeat_log_file="$repo_root/build/qemu-smoke-debugcon.log"

if ! command -v "$qemu" >/dev/null 2>&1; then
    echo "missing QEMU executable: $qemu" >&2
    exit 127
fi
if ! command -v timeout >/dev/null 2>&1; then
    echo "missing command: timeout" >&2
    exit 127
fi

make -C "$repo_root" build
if [[ ! -f "$repo_root/OVMF.fd" ]]; then
    echo "missing UEFI firmware: $repo_root/OVMF.fd (run ./build.sh setup)" >&2
    exit 1
fi
rm -f "$log_file"
rm -f "$heartbeat_log_file"

status=0
timeout --signal=TERM --kill-after=5s "${timeout_seconds}s" "$qemu" \
    -m 256M \
    -serial "file:$log_file" \
    -debugcon "file:$heartbeat_log_file" \
    -global isa-debugcon.iobase=0xe9 \
    -monitor none \
    -drive "if=pflash,format=raw,file=$repo_root/OVMF.fd,readonly=on" \
    -drive "file=$repo_root/build/hdd.img,format=raw" \
    -M q35 \
    -device e1000,netdev=net0 \
    -netdev user,id=net0 \
    -device nec-usb-xhci,id=xhci \
    -device usb-kbd,bus=xhci.0 \
    -device usb-mouse,bus=xhci.0 \
    -display none \
    -no-reboot || status=$?

if [[ "$status" -ne 0 && "$status" -ne 124 && "$status" -ne 137 ]]; then
    echo "QEMU exited unexpectedly with status $status" >&2
    [[ ! -f "$log_file" ]] || cat "$log_file" >&2
    exit "$status"
fi

for marker in \
    "[BOOT] Step 1: serial OK" \
    "[BOOT] Step 9: Filesystem" \
    "[TIMER] Clock source:" \
    "[TIMER] RTC (BIOS):" \
    "[VMM] Initialization complete!" \
    "vibra:/>"; do
    if ! grep -Fq "$marker" "$log_file"; then
        echo "boot smoke test failed: missing marker: $marker" >&2
        [[ ! -f "$log_file" ]] || cat "$log_file" >&2
        exit 1
    fi
done

if grep -Fq "[FATAL]" "$log_file"; then
    echo "boot smoke test failed: fatal boot message found" >&2
    cat "$log_file" >&2
    exit 1
fi

heartbeat_count="$(grep -Fc "[HEARTBEAT]" "$heartbeat_log_file" || true)"
if (( heartbeat_count < minimum_heartbeats )); then
    echo "boot smoke test failed: expected at least $minimum_heartbeats heartbeats, got $heartbeat_count" >&2
    cat "$log_file" >&2
    [[ ! -f "$heartbeat_log_file" ]] || cat "$heartbeat_log_file" >&2
    exit 1
fi

echo "QEMU boot smoke test passed; serial log: $log_file; heartbeat log: $heartbeat_log_file"
