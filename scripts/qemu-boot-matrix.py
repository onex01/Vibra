#!/usr/bin/env python3
import os
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
SOURCE_IMAGE = Path(os.environ.get("BOOT_SOURCE_IMAGE", BUILD / "hdd.img"))
FIRMWARE = ROOT / "OVMF.fd"
QEMU = os.environ.get("QEMU", "qemu-system-x86_64")
ITERATIONS = int(os.environ.get("BOOT_ITERATIONS", "100"))
BOOT_TIMEOUT = float(os.environ.get("BOOT_TIMEOUT", "20"))
BOOT_MARKERS = (
    b"[BOOT] Step 1: serial OK",
    b"[VMM] Initialization complete!",
)


def run_boot(disk_image: Path, debug_log: Path, iteration: int) -> None:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(BOOT_TIMEOUT)
    port = listener.getsockname()[1]

    command = [
        QEMU,
        "-m", "256M",
        "-serial", f"tcp:127.0.0.1:{port}",
        "-debugcon", f"file:{debug_log}",
        "-global", "isa-debugcon.iobase=0xe9",
        "-monitor", "none",
        "-drive", f"if=pflash,format=raw,file={FIRMWARE},readonly=on",
        "-drive", f"file={disk_image},format=raw,snapshot=on",
        "-M", "q35",
        "-device", "e1000,netdev=net0",
        "-netdev", "user,id=net0",
        "-device", "nec-usb-xhci,id=xhci",
        "-device", "usb-kbd,bus=xhci.0",
        "-device", "usb-mouse,bus=xhci.0",
        "-display", "none",
        "-no-reboot",
    ]
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    connection = None
    try:
        connection, _ = listener.accept()
        connection.settimeout(0.5)
        output = bytearray()
        deadline = time.monotonic() + BOOT_TIMEOUT
        while time.monotonic() < deadline:
            if b"[FATAL]" in output or b"!!! KERNEL PANIC !!!" in output:
                raise RuntimeError(f"boot {iteration} logged a fatal error:\n{output.decode('ascii', 'replace')[-2000:]}")
            debug_output = debug_log.read_bytes() if debug_log.exists() else b""
            if all(marker in output for marker in BOOT_MARKERS) and b"[HEARTBEAT]" in debug_output:
                return
            if process.poll() is not None:
                raise RuntimeError(f"QEMU exited during boot {iteration}: {process.returncode}")
            try:
                chunk = connection.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                raise RuntimeError(f"serial closed during boot {iteration}")
            output.extend(chunk)

        raise RuntimeError(f"boot {iteration} timed out:\n{output.decode('ascii', 'replace')[-2000:]}")
    finally:
        if connection is not None:
            connection.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            listener.close()


def main() -> None:
    if ITERATIONS < 1:
        raise SystemExit("BOOT_ITERATIONS must be positive")
    if not SOURCE_IMAGE.is_file() or not FIRMWARE.is_file():
        raise SystemExit("run ./build.sh setup and ./build.sh build first")

    with tempfile.TemporaryDirectory(prefix="vibra-boot-matrix-", dir=BUILD) as temp_dir:
        test_image = Path(temp_dir) / "hdd.img"
        shutil.copyfile(SOURCE_IMAGE, test_image)
        for iteration in range(1, ITERATIONS + 1):
            debug_log = Path(temp_dir) / f"debug-{iteration}.log"
            run_boot(test_image, debug_log, iteration)
            if iteration % 10 == 0 or iteration == ITERATIONS:
                print(f"QEMU boot matrix: {iteration}/{ITERATIONS} passed", flush=True)


if __name__ == "__main__":
    main()