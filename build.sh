#!/usr/bin/env bash
# Удобная точка входа для типовых сборок Vibra. Скрипт намеренно не запускает
# clean/setup автоматически: это сохраняет существующий диск и не перекачивает
# Limine/OVMF на каждом запуске.
set -eu

mode="${1:-debug}"
target="x86_64-unknown-none"

missing=0

have_cmd() {
    command -v "$1" >/dev/null 2>&1
}

need_cmd() {
    if ! have_cmd "$1"; then
        echo "missing: $1 ($2)" >&2
        missing=1
    fi
}

need_file() {
    if [ ! -f "$1" ]; then
        echo "missing: $1 ($2)" >&2
        missing=1
    fi
}

need_dir() {
    if [ ! -d "$1" ]; then
        echo "missing: $1 ($2)" >&2
        missing=1
    fi
}

need_nightly() {
    if ! have_cmd rustup; then
        echo "missing: rustup (install Rust via https://rustup.rs)" >&2
        missing=1
        return
    fi

    if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
        echo "missing: Rust nightly toolchain (run: rustup toolchain install nightly)" >&2
        missing=1
    fi

    if ! cargo +nightly --version >/dev/null 2>&1; then
        echo "missing: cargo +nightly (run: rustup toolchain install nightly)" >&2
        missing=1
    fi

    if ! rustup target list --toolchain nightly --installed 2>/dev/null | grep -q "^${target}$"; then
        echo "missing: Rust target ${target} (run: rustup target add ${target} --toolchain nightly)" >&2
        missing=1
    fi
}

print_install_help() {
    cat <<'EOF'

Install hints:
  Ubuntu/Debian:
    sudo apt update
    sudo apt install qemu-system-x86 mtools wget xorriso dosfstools fdisk make

  Rust:
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    rustup toolchain install nightly
    rustup target add x86_64-unknown-none --toolchain nightly

  First project setup:
    ./build.sh setup
EOF
}

preflight() {
    missing=0
    kind="${1:-build}"

    need_cmd make "required by build.sh"
    need_cmd grep "required by preflight checks"
    need_cmd dd "required for setup/usb images"
    need_cmd cp "required for ISO image staging"
    need_cmd rm "required for clean/setup"
    case "$kind" in
        setup)
            need_cmd wget "downloads Limine and OVMF"
            need_cmd tar "extracts Limine"
            need_cmd mkfs.fat "creates FAT32 boot image; package: dosfstools"
            need_cmd mmd "creates directories inside FAT image; package: mtools"
            need_cmd mcopy "copies files into FAT image; package: mtools"
            need_cmd mdir "verifies FAT image contents; package: mtools"
            ;;
        build)
            need_nightly
            need_cmd mcopy "copies kernel.elf into build/hdd.img; package: mtools"
            need_file build/hdd.img "run ./build.sh setup first"
            ;;
        run)
            need_nightly
            need_cmd qemu-system-x86_64 "runs Vibra in QEMU; package: qemu-system-x86"
            need_cmd mcopy "copies kernel.elf into build/hdd.img; package: mtools"
            need_file build/hdd.img "run ./build.sh setup first"
            need_file OVMF.fd "run ./build.sh setup first"
            ;;
        iso)
            need_nightly
            need_cmd xorriso "creates bootable ISO"
            need_dir limine-bin "run ./build.sh setup first"
            need_file OVMF.fd "run ./build.sh setup first"
            ;;
        usb)
            need_nightly
            need_cmd fdisk "creates USB image partition table"
            need_cmd mkfs.fat "formats USB partition; package: dosfstools"
            need_cmd mmd "creates directories inside USB FAT image; package: mtools"
            need_cmd mcopy "copies files into USB FAT image; package: mtools"
            need_dir limine-bin "run ./build.sh setup first"
            ;;
        doctor)
            need_nightly
            need_cmd qemu-system-x86_64 "runs Vibra in QEMU; package: qemu-system-x86"
            need_cmd wget "downloads Limine and OVMF"
            need_cmd tar "extracts Limine"
            need_cmd mkfs.fat "creates FAT32 images; package: dosfstools"
            need_cmd mmd "creates directories inside FAT images; package: mtools"
            need_cmd mcopy "copies files into FAT images; package: mtools"
            need_cmd mdir "verifies FAT images; package: mtools"
            need_cmd xorriso "creates bootable ISO"
            need_cmd fdisk "creates USB image partition table"
            ;;
    esac

    if [ "$missing" -ne 0 ]; then
        print_install_help
        exit 1
    fi
}

usage() {
    cat <<'EOF'
Usage: ./build.sh <mode>

ПРОФИЛИ СБОРКИ:
  debug         Development-образ и QEMU с serial shell (default)
  release       Optimized-сборка без debug-symbols
  secure        Optimized-образ без serial shell и QEMU
  build         Алиас для debug-build
  debug-build   Только development-сборка kernel.elf
  release-build Только optimized-сборка kernel.elf
  secure-build  Только защищённая сборка kernel.elf (no serial-debug)
  doctor        Проверить системные зависимости без сборки
  iso           Собрать bootable ISO
  run-iso       Запустить ISO через UEFI/OVMF
  run-bios      Запустить ISO через SeaBIOS
  qemu-usb      Запустить QEMU с xHCI USB keyboard/mouse
  usb           Собрать USB flash image
  setup         Создать загрузочный FAT-образ и скачать Limine/OVMF при необходимости
  clean         Очистить artefacts сборки и загрузочный образ
  help          Показать эту справку

ОПИСАНИЕ:
  development (debug)    - Сerial shell включён, отладка через COM1
  release                - Оптимизированная сборка без serial shell
  secure                 - Optimized без serial shell (для релиза)
  build                  - Алиас debug-build
  debug-build            - Только сборка development версии
  release-build          - Только сборка release версии
  secure-build           - Только сборка без serial-debug
  doctor                 - Проверить пакеты, Rust nightly и boot image
  setup                  - Создать диск и загрузчики (первый запуск)
  clean                  - Удалить build/ и target/

ПЕРЕМЕННЫЕ ОКРУЖЕНИЯ:
  SERIAL_DEBUG=0         - Отключить serial shell (для production)
  QEMU_OPTS="-m 512M"    - Дополнительные флаги QEMU

ПРИМЕРЫ:
  ./build.sh debug       # development + QEMU (графика + COM1)
  ./build.sh release     # optimized + QEMU (только графика)
  ./build.sh secure      # secure + QEMU (без COM1-ввода)
  ./build.sh debug-build # только development-сборка
  ./build.sh doctor      # проверить окружение
  ./build.sh setup       # первый запуск (скачать загрузчики)

Перед первым запуском выполните: ./build.sh setup
EOF
}

case "$mode" in
    debug)
        preflight run
        make run
        ;;
    release)
        preflight run
        make run-release
        ;;
    secure)
        preflight run
        make run SERIAL_DEBUG=0
        ;;
    debug-build)
        preflight build
        make build
        ;;
    build)
        preflight build
        make build
        ;;
    release-build)
        preflight build
        make build-release
        ;;
    secure-build)
        preflight build
        make build SERIAL_DEBUG=0
        ;;
    doctor)
        preflight doctor
        echo "OK: build environment looks ready"
        ;;
    setup)
        preflight setup
        make setup
        ;;
    iso)
        preflight iso
        make iso
        ;;
    run-iso)
        preflight iso
        make run-iso
        ;;
    run-bios)
        preflight iso
        make run-bios
        ;;
    qemu-usb)
        preflight run
        make qemu-usb
        ;;
    usb)
        preflight usb
        make usb
        ;;
    clean)
        make clean
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "Unknown build mode: $mode" >&2
        usage >&2
        exit 2
        ;;
esac
