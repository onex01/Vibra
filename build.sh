#!/usr/bin/env bash
# Удобная точка входа для типовых сборок Vibra. Скрипт намеренно не запускает
# clean/setup автоматически: это сохраняет существующий диск и не перекачивает
# Limine/OVMF на каждом запуске.
set -eu

mode="${1:-debug}"

usage() {
    cat <<'EOF'
Usage: ./build.sh <mode>

ПРОФИЛИ СБОРКИ:
  debug         Development-образ и QEMU с serial shell (default)
  release       Optimized-сборка без debug-symbols
  secure        Optimized-образ без serial shell и QEMU
  debug-build   Только development-сборка kernel.elf
  release-build Только optimized-сборка kernel.elf
  secure-build  Только защищённая сборка kernel.elf (no serial-debug)
  setup         Создать загрузочный FAT-образ и скачать Limine/OVMF при необходимости
  clean         Очистить artefacts сборки и загрузочный образ
  help          Показать эту справку

ОПИСАНИЕ:
  development (debug)    - Сerial shell включён, отладка через COM1
  release                - Оптимизированная сборка без serial shell
  secure                 - Optimized без serial shell (для релиза)
  debug-build            - Только сборка development версии
  release-build          - Только сборка release версии
  secure-build           - Только сборка без serial-debug
  setup                  - Создать диск и загрузчики (первый запуск)
  clean                  - Удалить build/ и target/

ПЕРЕМЕННЫЕ ОКРУЖЕНИЯ:
  SERIAL_DEBUG=0         - Отключить serial shell (для production)
  QEMU_OPTS="-m 512M"    - Дополнительные флаги QEMU

ПРИМЕРЫ:
  ./build.sh debug       # development + QEMU (графика + COM1)
  ./build.sh release     # optimized + QEMU (только графика)
  ./build.sh secure      # secure + QEMU (без COM1-ввода)
  ./build.sh build       # только development-сборка
  ./build.sh setup       # первый запуск (скачать загрузчики)

Перед первым запуском выполните: ./build.sh setup
EOF
}

case "$mode" in
    debug)
        make run
        ;;
    release)
        make run QEMU_OPTS="-m 256M -serial none"
        ;;
    secure)
        make run SERIAL_DEBUG=0
        ;;
    debug-build)
        make build
        ;;
    release-build)
        make build QEMU_OPTS=""
        ;;
    secure-build)
        make build SERIAL_DEBUG=0
        ;;
    setup)
        make setup
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
