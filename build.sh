#!/usr/bin/env bash
# build.sh — host-side entry point for Vibra Kernel.
#
# Two interfaces over the same command table:
#
#   ./build.sh <command> [args]   non-interactive (used by CI and scripts)
#   ./build.sh                    interactive menu
#   ./build.sh menu               force the menu
#
# The UI language is English on purpose: menus are printed to terminals with
# arbitrary locales, and a half-translated prompt is worse than no prompt.
# Functional output from make/cargo stays as it is.
set -u

# ---------------------------------------------------------------------------
# Host detection and preflight
# ---------------------------------------------------------------------------

target="x86_64-unknown-none"
missing=0

have() { command -v "$1" >/dev/null 2>&1; }
need() {
    if ! have "$1"; then
        echo "  missing: $1 ($2)" >&2
        missing=1
    fi
}
need_file() {
    if [ ! -f "$1" ]; then
        echo "  missing: $1 ($2)" >&2
        missing=1
    fi
}
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
        echo "Install dependencies on the host, then run build.sh from a native terminal." >&2
    fi
}

install_hint() {
    case "$(host_family)" in
        debian) echo "sudo apt update && sudo apt install make qemu-system-x86 mtools dosfstools wget tar xorriso fdisk ntfs-3g e2fsprogs python3" ;;
        fedora) echo "sudo dnf install make qemu-system-x86 mtools dosfstools wget tar xorriso util-linux ntfs-3g e2fsprogs python3" ;;
        *) echo "Install: make qemu-system-x86 mtools dosfstools wget tar xorriso fdisk/util-linux ntfs-3g e2fsprogs python3" ;;
    esac
}

rust_hint() {
    echo "Install Rust (apt/dnf rustup packages are broken; use the official installer):" >&2
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    echo '  source "$HOME/.cargo/env"' >&2
    echo "  rustup toolchain install nightly --component rust-src" >&2
    echo "  rustup target add $target --toolchain nightly" >&2
}

need_rust() {
    need rustup "install Rust with rustup"
    if have rustup; then
        if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
            echo "  missing: Rust nightly" >&2
            missing=1
        fi
        if ! rustup target list --toolchain nightly --installed 2>/dev/null | grep -qx "$target"; then
            echo "  missing: Rust target $target for nightly" >&2
            missing=1
        fi
        if ! rustup component list --toolchain nightly --installed 2>/dev/null | grep -qx "rust-src"; then
            echo "  missing: Rust component rust-src for nightly (-Z build-std)" >&2
            missing=1
        fi
    fi
}

# Packages required by a given group of commands. Keeps CI and the menu in
# sync: both go through preflight().
require_packages() {
    case "$1" in
        doctor)
            need qemu-system-x86_64 "run QEMU"
            need wget "download Limine/OVMF"
            need tar "extract Limine"
            need mkfs.fat "format FAT image"
            need mmd "create image directories"
            need mcopy "copy boot files"
            need xorriso "create ISO"
            need fdisk "partition USB image"
            ;;
        setup)
            need wget "download Limine/OVMF"
            need tar "extract Limine"
            need dd "create image"
            need mkfs.fat "format FAT image"
            need mmd "create image directories"
            need mcopy "copy boot files"
            ;;
        build)
            need mcopy "copy kernel into image"
            need_file build/hdd.img "run './build.sh setup' first"
            ;;
        run)
            need qemu-system-x86_64 "run QEMU"
            need mcopy "copy kernel into image"
            need_file build/hdd.img "run './build.sh setup' first"
            need_file OVMF.fd "run './build.sh setup' first"
            ;;
        iso)
            need xorriso "create ISO"
            need mcopy "copy kernel"
            need_file build/hdd.img "run './build.sh setup' first"
            ;;
        iso-smoke)
            need qemu-system-x86_64 "run QEMU"
            need xorriso "create ISO"
            need mcopy "copy kernel"
            need_file build/hdd.img "run './build.sh setup' first"
            need_file OVMF.fd "run './build.sh setup' first"
            ;;
        usb)
            need fdisk "partition USB image"
            need mkfs.fat "format FAT image"
            need mmd "create image directories"
            need mcopy "copy boot files"
            need_file build/hdd.img "run './build.sh setup' first"
            ;;
        uefi-usb)
            need_file limine-bin/BOOTX64.EFI "run './build.sh setup' first"
            ;;
        "" ) : ;;
    esac
}

preflight() {
    local kind="${1:-}"
    missing=0
    need make "build orchestration"
    need_rust
    require_packages "$kind"
    if [ "$missing" -ne 0 ]; then
        echo >&2
        echo "Host family: $(host_family)" >&2
        flatpak_notice
        echo "Install missing host packages:" >&2
        install_hint >&2
        rust_hint
        return 1
    fi
    return 0
}

# ---------------------------------------------------------------------------
# Menu
# ---------------------------------------------------------------------------

# Drawing helpers. No colour unless the terminal is a TTY: piped output (CI,
# logs) would otherwise be full of escape sequences.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    bold=$'\033[1m'; dim=$'\033[2m'; cyan=$'\033[36m'; green=$'\033[32m'
    yellow=$'\033[33m'; reset=$'\033[0m'
else
    bold=""; dim=""; cyan=""; green=""; yellow=""; reset=""
fi

# Version is read from the single source of truth, same as the Makefile.
version() {
    sed -n 's/^pub const KERNEL_VERSION: &str = "\(.*\)";/\1/p' kernel/src/version.rs 2>/dev/null
}

# Short host summary for the header: OS family, tools that are missing.
host_summary() {
    local family rust qemu
    family="$(host_family)"
    rust="$(rustup --version 2>/dev/null | head -n1 || true)"
    qemu="$(qemu-system-x86_64 --version 2>/dev/null | head -n1 | cut -d' ' -f1-2 || true)"
    printf 'host: %s  rust: %s  qemu: %s' \
        "$family" "${rust:-not found}" "${qemu:-not found}"
}

banner() {
    local v
    v="$(version)"
    printf '%sVibra Kernel%s %s\n' "$bold" "$reset" "${v:-<unknown version>}"
    printf '%s%s%s\n' "$dim" "$(host_summary)" "$reset"
}

hr() { printf '%s%s%s\n' "$dim" "$(printf '%.0s-' $(seq 1 72))" "$reset"; }

# Pause only when interactive: piped runs must not block.
pause() {
    if [ -t 0 ]; then
        printf '\n%sPress Enter to return to the menu...%s' "$dim" "$reset"
        read -r _ || true
    fi
}

menu_build() {
    while :; do
        printf '\n%sBuild%s\n' "$bold" "$reset"
        hr
        cat <<'MENU'
  1) Kernel, debug profile (opt-level 1, serial on)
  2) Kernel, release profile (opt-level 3, no serial)
  3) Check warning budgets (debug + release)
  4) Clean build directory
  5) Back
MENU
        printf 'Choice: '
        read -r choice || return
        case "$choice" in
            1) run_command build ;;
            2) run_command build-release ;;
            3) run_command check ;;
            4) run_command clean ;;
            5) return ;;
            *) echo "Unknown choice: $choice" ;;
        esac
        [ "${choice:-}" = "5" ] || pause
    done
}

menu_run() {
    while :; do
        printf '\n%sRun in QEMU%s\n' "$bold" "$reset"
        hr
        cat <<'MENU'
  1) Debug kernel with serial console on stdio
  2) Release kernel (fast, no serial)
  3) Debug kernel with serial over TCP (port 4444)
  4) Boot the ISO through SeaBIOS (BIOS path)
  5) Back
MENU
        printf 'Choice: '
        read -r choice || return
        case "$choice" in
            1) run_command debug ;;
            2) run_command release ;;
            3) run_command run-serial-tcp ;;
            4) run_command run-bios ;;
            5) return ;;
            *) echo "Unknown choice: $choice" ;;
        esac
        [ "${choice:-}" = "5" ] || pause
    done
}

menu_images() {
    while :; do
        printf '\n%sImages%s\n' "$bold" "$reset"
        hr
        printf '%sDebug images keep the VIBRALOG partition and /VIBRA.LOG,\nso they are the ones to use when something fails on real hardware.\nThe "noserial" variants drop COM1: same logging, but no 87 us per\ncharacter wait on every write.%s\n\n' "$dim" "$reset"
        cat <<'MENU'
  1) ISO, debug + serial      vibra-debug_*.iso
  2) ISO, debug, no serial    vibra-debug-noserial_*.iso
  3) ISO, release             vibra-release_*.iso
  4) USB image, debug          vibra-usb-debug_*.img
  5) USB image, debug, no serial  vibra-usb-debug-noserial_*.img
  6) USB image, BIOS/CSM       vibra-usb_*.img
  7) USB image, UEFI only      vibra-uefi_*.img
  8) All debug variants at once
  9) Release bundle (all images + checksums + notes) -> build/dist
 10) Back
MENU
        printf 'Choice: '
        read -r choice || return
        case "$choice" in
            1) run_command iso-debug ;;
            2) run_command iso-debug-noserial ;;
            3) run_command iso-release ;;
            4) run_command usb-debug ;;
            5) run_command usb-debug-noserial ;;
            6) run_command usb ;;
            7) run_command uefi-usb ;;
            8) run_command debug-variants ;;
            9) run_command dist ;;
            10) return ;;
            *) echo "Unknown choice: $choice" ;;
        esac
        [ "${choice:-}" = "10" ] || pause
    done
}

menu_tests() {
    while :; do
        printf '\n%sTests%s\n' "$bold" "$reset"
        hr
        cat <<'MENU'
Boot and console
  1) Smoke boot (UEFI)                      smoke
  2) Boot without a COM port                noserial-test
  3) Input through COM1                     serial-input-test
  4) Interactive shell (Tab, history)       shell-test
  5) VK tests inside the kernel             vktst-test
  6) Keyboard (PS/2 and USB HID)            keyboard-test

Storage
  7) FAT32 / ext2 / ext4 / NTFS / MBR       fs-test
  8) Large volumes, 4Kn, exFAT, labels      big-fs-test
  9) NVMe                                   nvme-test
 10) ISO9660 from a CD-ROM                  iso-test
 11) Read-only mount, mount/umount/sync     ro-mount-test
 12) Corrupted filesystems                  bad-fs-test
 13) Boot log written to VIBRALOG           boot-log-test
 14) USB storage through the VFS            usb-storage-test
 15) UHCI (USB 2.0) keyboard                uhci-test

Other
 16) Clock and timers                       clock-test
 17) RAM filesystem                         ramfs-test
 18) Debug image with three partitions      debug-image-test
 19) Debug ISO written to USB               iso-debug-test
 20) ISO boot matrix (UEFI + BIOS)          iso-smoke
 21) USB3 (SuperSpeed) disk and hotplug     usb3-test
 22) EHCI (USB 2.0) disk and hotplug        ehci-test
 23) Back
MENU
        printf 'Choice: '
        read -r choice || return
        case "$choice" in
            1) run_command smoke ;;
            2) run_command noserial-test ;;
            3) run_command serial-input-test ;;
            4) run_command shell-test ;;
            5) run_command vktst-test ;;
            6) run_command keyboard-test ;;
            7) run_command fs-test ;;
            8) run_command big-fs-test ;;
            9) run_command nvme-test ;;
            10) run_command iso-test ;;
            11) run_command ro-mount-test ;;
            12) run_command bad-fs-test ;;
            13) run_command boot-log-test ;;
            14) run_command usb-storage-test ;;
            15) run_command uhci-test ;;
            16) run_command clock-test ;;
            17) run_command ramfs-test ;;
            18) run_command debug-image-test ;;
            19) run_command iso-debug-test ;;
            20) run_command iso-smoke ;;
            21) run_command usb3-test ;;
            22) run_command ehci-test ;;
            23) return ;;
            *) echo "Unknown choice: $choice" ;;
        esac
        [ "${choice:-}" = "23" ] || pause
    done
}

menu_tools() {
    while :; do
        printf '\n%sDiagnostics and tools%s\n' "$bold" "$reset"
        hr
        cat <<'MENU'
  1) Check host environment                   doctor
  2) Download Limine + OVMF, create hdd.img   setup
  3) Run the debug ISO in QEMU                run-iso
  4) Read the boot log from a device          read-boot-log
  5) Show release notes for this version      release-notes
  6) List every command                       list
  7) Back
MENU
        printf 'Choice: '
        read -r choice || return
        case "$choice" in
            1) run_command doctor ;;
            2) run_command setup ;;
            3) run_command run-iso ;;
            4) run_command read-boot-log ;;
            5) run_command release-notes ;;
            6) list_commands ;;
            7) return ;;
            *) echo "Unknown choice: $choice" ;;
        esac
        [ "${choice:-}" = "7" ] || pause
    done
}

menu_main() {
    while :; do
        clear 2>/dev/null || true
        banner
        printf '\n%sWhat do you want to do?%s\n' "$bold" "$reset"
        hr
        cat <<'MENU'
  1) Build the kernel
  2) Run it in QEMU
  3) Build images (ISO, USB, debug with / without serial)
  4) Run tests
  5) Diagnostics and tools
  6) Quit
MENU
        printf 'Choice: '
        # EOF (закрытый stdin) — это выход, а не ошибка: иначе запуск из
        # скрипта или `</dev/null` завершался бы с кодом 1.
        read -r choice || exit 0
        case "$choice" in
            1) menu_build ;;
            2) menu_run ;;
            3) menu_images ;;
            4) menu_tests ;;
            5) menu_tools ;;
            6|q|Q) printf 'Bye.\n'; exit 0 ;;
            '') : ;;
            *) echo "Unknown choice: $choice" ;;
        esac
    done
}

# ---------------------------------------------------------------------------
# Command table: name -> "preflight-kind|make target|extra make args"
#
# One entry per command keeps the dispatch, the help text and the menu from
# drifting apart: `list` prints this table, and the menu calls the same
# run_command path that the CLI uses.
# ---------------------------------------------------------------------------

command_table() {
    cat <<'TABLE'
doctor|doctor|-|
setup|setup|setup|
build|build|build|
build-release|run|build|RELEASE=1 SERIAL_DEBUG=0
check|build|check|
build-script-test|build|build-script-test|
clean||clean|
debug|run|run|
release|run|run|RELEASE=1 SERIAL_DEBUG=0
run-serial-tcp|run|run|SERIAL_MODE=tcp
run-serial-file|run|run|SERIAL_MODE=file
run-bios|run|run-bios|
run-usb|run|run-usb|
run-iso|iso|run-iso|
run-iso-usb|iso|run-iso-usb|
qemu-usb|run|qemu-usb|
iso|iso|iso|
iso-release|iso|iso-release|
iso-debug|iso|iso-debug|
iso-debug-noserial|iso|iso-debug-noserial|
iso-smoke|iso-smoke|iso-smoke|
usb|usb|usb|
usb-debug|iso|usb-debug|
usb-debug-noserial|iso|usb-debug-noserial|
uefi-usb|uefi-usb|uefi-usb|
debug-variants|iso|debug-variants|
dist|iso|dist|
release-notes||release-notes|
smoke|run|smoke|
clock-test|build|clock-test|
ramfs-test|build|ramfs-test|
ro-mount-test|build|ro-mount-test|
bad-fs-test|build|bad-fs-test|
nvme-test|build|nvme-test|
big-fs-test|build|big-fs-test|
iso-test|build|iso-test|
usb3-test|build|usb3-test|
ehci-test|build|ehci-test|
noserial-test|build|noserial-test|
serial-input-test|build|serial-input-test|
shell-test|build|shell-test|
fs-test|build|fs-test|
keyboard-test|run|keyboard-test|
uhci-test|run|uhci-test|
vktst-test|build|vktst-test|
boot-log-test|build|boot-log-test|
usb-storage-test|run|usb-storage-test|
debug-image-test|usb|debug-image-test|
iso-debug-test|iso|iso-debug-test|
read-boot-log||read-boot-log|
TABLE
}

command_exists() {
    command_table | grep -q "^${1}|"
}

# Execute one command by name. Shared by the CLI and the menu.
run_command() {
    local name="$1"; shift
    # Variable names must not shadow the globals: `target` is the Rust target
    # triple used by the preflight (a local `target` holding a make target name
    # made it run `rustup target add build`).
    local line kind make_target make_args
    line="$(command_table | grep -m1 "^${name}|")"
    if [ -z "$line" ]; then
        echo "Unknown command: $name" >&2
        return 2
    fi
    IFS='|' read -r _ kind make_target make_args <<<"$line"

    if ! preflight "$kind"; then
        return 1
    fi

    # `-` means the command has no make target: it only runs host checks.
    if [ "$make_target" = "-" ]; then
        printf '\n%sOK%s: %s host is ready for Vibra Kernel\n' "$green" "$reset" "$(host_family)"
        return 0
    fi

    printf '\n%s==> %s%s\n' "$cyan" "$name" "$reset"
    # Extra make variables (RELEASE=1, SERIAL_MODE=...) come from the table.
    # DRY_RUN=1 prints the resolved make command instead of running it; the
    # project's own test script uses it to check that every command in the
    # table maps to a real target.
    # shellcheck disable=SC2086
    if [ "${DRY_RUN:-0}" = "1" ]; then
        echo "make ${make_target}${make_args:+ $make_args}${*:+ $*}"
        return 0
    fi
    make $make_target $make_args "$@"
}

list_commands() {
    printf '\n%sCommands%s (same table the menu uses)\n' "$bold" "$reset"
    hr
    command_table | while IFS='|' read -r name _kind _target _args; do
        printf '  %-20s\n' "$name"
    done
    printf '\nUsage: ./build.sh <command> [args]   or   ./build.sh   for the menu\n'
}

usage() {
    cat <<'USAGE'
Usage: ./build.sh [command] [args]

Run with no command to get an interactive menu.

Environment:
  QEMU=...            QEMU binary (default: qemu-system-x86_64)
  NO_COLOR=1          disable coloured output
  DRY_RUN=1           print the make command instead of running it
  LOG_DEVICE=/dev/sdX device for 'read-boot-log'

Commands are listed by: ./build.sh list
USAGE
}

# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

if [ $# -eq 0 ]; then
    menu_main
    exit 0
fi

mode="$1"
shift || true

case "$mode" in
    menu) menu_main ;;
    list|commands) list_commands ;;
    help|-h|--help) usage ;;
    read-boot-log)
        # Device may come either as the first argument or from the environment.
        if [ $# -gt 0 ]; then
            make read-boot-log LOG_DEVICE="$1"
        else
            make read-boot-log LOG_DEVICE="${LOG_DEVICE:-}"
        fi
        ;;
    *)
        if command_exists "$mode"; then
            run_command "$mode" "$@"
        else
            echo "Unknown command: $mode" >&2
            echo >&2
            usage >&2
            exit 2
        fi
        ;;
esac
