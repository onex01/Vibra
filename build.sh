#!/usr/bin/env bash
set -eu
mode="${1:-debug}"
target="x86_64-unknown-none"
missing=0
have() { command -v "$1" >/dev/null 2>&1; }
need() { if ! have "$1"; then echo "missing: $1 ($2)" >&2; missing=1; fi; }
need_file() { if [ ! -f "$1" ]; then echo "missing: $1 ($2)" >&2; missing=1; fi; }
is_flatpak() { [ -f /.flatpak-info ]; }
host_os_release() {
    if is_flatpak && have flatpak-spawn; then
        flatpak-spawn --host sh -c 'cat /etc/os-release' 2>/dev/null || true
    elif [ -r /etc/os-release ]; then
        cat /etc/os-release
    fi
}
host_family() {
    local os_release id like
    os_release="$(host_os_release)"
    id="$(printf '%s\n' "$os_release" | sed -n 's/^ID=//p' | head -n 1 | tr -d '"')"
    like="$(printf '%s\n' "$os_release" | sed -n 's/^ID_LIKE=//p' | head -n 1 | tr -d '"')"
    case "$id $like" in
        *debian*|*ubuntu*) echo debian ;;
        *fedora*|*rhel*|*centos*|*rocky*|*alma*) echo fedora ;;
        *) echo unknown ;;
    esac
}
flatpak_notice() {
    if is_flatpak; then
        echo "Note: this terminal runs inside a Flatpak sandbox." >&2
        echo "Host packages and QEMU are intentionally hidden from it." >&2
        echo "Install dependencies on the host, then run build.sh from a native host terminal." >&2
    fi
}
preflight_uefi_usb() { missing=0; need make "build orchestration"; need_rust; need_file limine-bin/BOOTX64.EFI "run ./build.sh setup first"; if [ "$missing" -ne 0 ]; then echo >&2; echo "Host family: $(host_family)" >&2; flatpak_notice; echo "Install missing host packages:" >&2; install_hint >&2; rust_hint; exit 1; fi; }
install_hint() { case "$(host_family)" in debian) echo "sudo apt update && sudo apt install make qemu-system-x86 mtools dosfstools wget tar xorriso fdisk" ;; fedora) echo "sudo dnf install make qemu-system-x86 mtools dosfstools wget tar xorriso util-linux" ;; *) echo "Install: make qemu-system-x86 mtools dosfstools wget tar xorriso fdisk/util-linux" ;; esac; }
need_rust() { need rustup "install Rust with rustup"; if have rustup; then if ! rustup toolchain list 2>/dev/null | grep -q "^nightly"; then echo "missing: Rust nightly" >&2; missing=1; fi; if ! rustup target list --toolchain nightly --installed 2>/dev/null | grep -qx "$target"; then echo "missing: Rust target $target for nightly" >&2; missing=1; fi; if ! rustup component list --toolchain nightly --installed 2>/dev/null | grep -qx "rust-src"; then echo "missing: Rust component rust-src for nightly (-Z build-std)" >&2; missing=1; fi; fi; }
rust_hint() { echo "Install Rust: пакеты rustup/rust из apt/dnf не дают рабочего rustup — используйте официальный установщик:" >&2; echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2; echo "  source \"\$HOME/.cargo/env\"" >&2; echo "  rustup toolchain install nightly --component rust-src" >&2; echo "  rustup target add $target --toolchain nightly" >&2; }
preflight() { kind="$1"; missing=0; need make "build orchestration"; need_rust; case "$kind" in doctor) need qemu-system-x86_64 "run QEMU"; need wget "download Limine/OVMF"; need tar "extract Limine"; need mkfs.fat "format FAT image"; need mmd "create image directories"; need mcopy "copy boot files"; need xorriso "create ISO"; need fdisk "partition USB image" ;; setup) need wget "download Limine/OVMF"; need tar "extract Limine"; need dd "create image"; need mkfs.fat "format FAT image"; need mmd "create image directories"; need mcopy "copy boot files" ;; build) need mcopy "copy kernel into image"; need_file build/hdd.img "run ./build.sh setup first" ;; run) need qemu-system-x86_64 "run QEMU"; need mcopy "copy kernel into image"; need_file build/hdd.img "run ./build.sh setup first"; need_file OVMF.fd "run ./build.sh setup first" ;; iso) need xorriso "create ISO"; need mcopy "copy kernel"; need_file build/hdd.img "run ./build.sh setup first" ;; iso-smoke) need qemu-system-x86_64 "run QEMU"; need xorriso "create ISO"; need mcopy "copy kernel"; need_file build/hdd.img "run ./build.sh setup first"; need_file OVMF.fd "run ./build.sh setup first" ;; usb) need fdisk "partition USB image"; need mkfs.fat "format FAT image"; need mmd "create image directories"; need mcopy "copy boot files"; need_file build/hdd.img "run ./build.sh setup first" ;; esac; if [ "$missing" -ne 0 ]; then echo >&2; echo "Host family: $(host_family)" >&2; flatpak_notice; echo "Install missing host packages:" >&2; install_hint >&2; rust_hint; exit 1; fi; }
usage() { printf "%s\n" "Usage: ./build.sh <command>" "  doctor | setup | build | smoke | clock-test | fs-test | keyboard-test | iso | iso-smoke" "  boot-log-test | usb-storage-test | usb-debug | iso-debug | read-boot-log" "  debug | release | secure | usb | uefi-usb | qemu-usb | clean"; }
if [[ "$mode" == "uefi-usb" ]]; then
    preflight_uefi_usb
    make uefi-usb
    exit $?
fi
case "$mode" in doctor) preflight doctor; echo "OK: $(host_family) host is ready for Vibra Kernel" ;; setup) preflight setup; make setup ;; build|debug-build) preflight build; make build ;; smoke) preflight run; make smoke ;; clock-test) preflight build; make clock-test ;; fs-test) preflight build; make fs-test ;; keyboard-test) preflight run; make keyboard-test ;; iso-smoke) preflight iso-smoke; make iso-smoke ;; boot-log-test) preflight build; make boot-log-test ;; usb-storage-test) preflight run; make usb-storage-test ;; usb-debug) preflight iso; make usb-debug ;; iso-debug) preflight iso; make iso-debug ;; read-boot-log) make read-boot-log LOG_DEVICE="${2:-${LOG_DEVICE:-}}" ;; debug) preflight run; make run ;; release|secure) preflight run; make run SERIAL_DEBUG=0 ;; iso) preflight iso; make iso ;; usb) preflight usb; make usb ;; qemu-usb) preflight run; make qemu-usb ;; clean) make clean ;; help|-h|--help) usage ;; *) echo "Unknown command: $mode" >&2; usage >&2; exit 2 ;; esac
