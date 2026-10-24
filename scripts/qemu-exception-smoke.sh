#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
qemu="${QEMU:-qemu-system-x86_64}"
test_name="${1:-}"

case "$test_name" in
    panic-test)
        command="diag panic-test"
        markers=("VIBRA KERNEL STOP SCREEN" "Stop code: KERNEL_PANIC" "[PANIC] RSP=" "RFLAGS=" "[PANIC] Location:" "diagnostic panic screen test")
        ;;
    fault-test)
        command="diag fault-test"
        markers=("!!! PAGE FAULT !!!" "Address (CR2) = " "Error=0x3: protection violation | write | kernel" "RIP=" "Reserved-bit violation=false" "[FATAL] Kernel page fault")
        ;;
    *)
        echo "Usage: $0 {panic-test|fault-test}" >&2
        exit 2
        ;;
esac

for tool in "$qemu" mcopy python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 127
    fi
done

source_image="${BOOT_SOURCE_IMAGE:-$repo_root/build/hdd.img}"
kernel="$repo_root/target/x86_64-unknown-none/debug/vibra"
firmware="$repo_root/OVMF.fd"
for file in "$source_image" "$kernel" "$firmware"; do
    if [[ ! -f "$file" ]]; then
        echo "missing required file: $file (run ./build.sh smoke first)" >&2
        exit 1
    fi
done

disk_image="$repo_root/build/qemu-exception-$$.img"
cp "$source_image" "$disk_image"
trap 'rm -f "$disk_image"' EXIT
mcopy -o -i "$disk_image" "$kernel" ::/kernel.elf

python3 - "$qemu" "$disk_image" "$firmware" "$command" "${markers[@]}" <<'PY'
import os
import socket
import subprocess
import sys
import time

qemu, disk, firmware, command, *markers = sys.argv[1:]
ready_timeout = float(os.environ.get("BOOT_READY_TIMEOUT", "45"))
marker_timeout = float(os.environ.get("MARKER_TIMEOUT", "15"))

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(1)
listener.settimeout(ready_timeout)
port = listener.getsockname()[1]

qemu_args = [
    qemu, "-m", "256M",
    "-serial", f"tcp:127.0.0.1:{port}",
    "-monitor", "none",
    "-drive", f"if=pflash,format=raw,file={firmware},readonly=on",
    "-drive", f"file={disk},format=raw",
    "-M", "q35",
    "-device", "e1000,netdev=net0",
    "-netdev", "user,id=net0",
    "-device", "nec-usb-xhci,id=xhci",
    "-device", "usb-kbd,bus=xhci.0",
    "-device", "usb-mouse,bus=xhci.0",
    "-display", "none",
    "-no-reboot",
]
process = subprocess.Popen(qemu_args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
connection = None
try:
    connection, _ = listener.accept()
    connection.settimeout(0.5)
    captured = bytearray()

    def receive_until(predicate, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return True
            try:
                chunk = connection.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                return False
            captured.extend(chunk)
        return predicate()

    if not receive_until(lambda: b"vibra:/>" in captured, ready_timeout):
        raise RuntimeError("guest shell did not become ready")

    connection.sendall(command.encode("ascii") + b"\r")
    if not receive_until(lambda: all(marker.encode() in captured for marker in markers), marker_timeout):
        text = captured.decode("ascii", "replace")
        missing = [marker for marker in markers if marker.encode() not in captured]
        raise RuntimeError(f"missing markers {missing!r}:\n" + text[-5000:])

    text = captured.decode("ascii", "replace")
    for line in text.splitlines():
        if any(marker.encode() in captured and marker in line for marker in markers):
            print(line)
    print(f"QEMU {command}: PASS")
finally:
    if connection is not None:
        connection.close()
    listener.close()
    process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
PY