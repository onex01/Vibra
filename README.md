<h1 align="center">🌊 Vibra OS</h1>

<p align="center">
  <b>Open Source операционная система, написанная с нуля на Rust</b>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.1.0%20Sirius-blue?style=flat-square" alt="Version"/>
  <img src="https://img.shields.io/badge/kernel-1.1.0%20Boson-green?style=flat-square" alt="Kernel"/>
  <img src="https://img.shields.io/badge/arch-x86__64-orange?style=flat-square" alt="Architecture"/>
  <img src="https://img.shields.io/badge/language-Rust-red?style=flat-square" alt="Rust"/>
  <img src="https://img.shields.io/badge/license-MIT-yellow?style=flat-square" alt="License"/>
</p>

---

## 📖 О проекте

**Vibra OS** — это хобби-операционная система, разрабатываемая с нуля на языке **Rust**. Проект создан с образовательными целями: изучение низкоуровневого программирования, архитектуры операционных систем и bare-metal разработки.

Vibra ориентируется на QEMU и реальное x86_64-железо через Limine. Сейчас система предоставляет графическую консоль, shell с автодополнением, VFS, RamFS, FAT32/Ext2-драйверы, базовый GUI/desktop и набор низкоуровневых драйверов.

### Основные особенности

- 🔧 **Написано с нуля** — ни строчки чужого кода в ядре
- 🦀 **Чистый Rust** (no_std) — без зависимостей от стандартной библиотеки
- 🚀 **UEFI-загрузка** через современный загрузчик Limine
- 🖥️ **Графическая консоль** с bitmap-шрифтом и поддержкой цветов
- 📁 **VFS + файловые системы** — RamFS, procfs/sysfs/devtmpfs, FAT32, Ext2
- 🧠 **Подсистема памяти** — PMM, heap, paging, VMM и W^X/NX-диагностика
- ⚙️ **Планировщик и syscall** — kernel tasks, Ring 3, syscall/sysret
- 🧩 **Драйверы** — PCI, ACPI, AHCI, NVMe, e1000/RTL8139, VirtIO, USB detection
- 🎨 **Модульная архитектура** — каждая команда shell в отдельном файле

---

## 🚀 Быстрый старт

### Системные требования

- **ОС хоста:** Linux (Ubuntu 20.04+, Debian 11+, Fedora)
- **Rust:** nightly toolchain
- **QEMU:** 6.0+
- **Утилиты:** `make`, `qemu-system-x86_64`, `mtools`, `dosfstools`, `wget`, `tar`
- **Для ISO/USB:** `xorriso`, `fdisk`

### Установка зависимостей

**Ubuntu / Debian:**
```bash
sudo apt update
sudo apt install qemu-system-x86 mtools dosfstools wget xorriso fdisk make
```

**Rust nightly:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install nightly
rustup target add x86_64-unknown-none --toolchain nightly
```

### Проверка окружения

`build.sh` умеет проверять зависимости перед сборкой и объясняет, чего не хватает:

```bash
./build.sh doctor
```

Для первого запуска достаточно:

```bash
./build.sh setup   # скачивает Limine/OVMF и создаёт build/hdd.img
./build.sh debug   # собирает Vibra и запускает QEMU
```

### Сборка и запуск

```bash
# Клонировать репозиторий
git clone https://github.com/OneX01/Vibra.git
cd Vibra

# Первый запуск (скачивает Limine и OVMF, создаёт диск)
make setup

# Собрать и запустить
make run
```

Или использовать скрипт `build.sh`:
```bash
chmod +x build.sh
./build.sh setup          # только при первом запуске
./build.sh debug          # development-сборка + QEMU (serial shell включён)
./build.sh release        # optimized-сборка без serial shell
./build.sh secure         # secure + QEMU (без COM1-ввода)
./build.sh build          # только development-сборка
./build.sh release-build  # только release-сборка
./build.sh secure-build   # только защищённая сборка (no serial-debug)
./build.sh doctor         # проверить зависимости
./build.sh run-bios       # ISO через SeaBIOS
./build.sh qemu-usb       # QEMU с xHCI USB keyboard/mouse
```

### Управление в QEMU

- **Закрыть QEMU:** `Ctrl+A`, затем `X`
- **Ввод:** печатайте прямо в окне QEMU

### Serial debug shell

Development-сборка по умолчанию (флаг `serial-debug` в Cargo) зеркалит
framebuffer-консоль в COM1 и принимает shell-команды из терминала.
Это удобно для автоматизации проверок ядра. PS/2-клавиатура в окне QEMU
продолжает работать.

Для защищённого образа (без serial shell и COM1-ввода):

```bash
make run SERIAL_DEBUG=0
# или
./build.sh secure
```

### Профили сборки

| Профиль         | SERIAL_DEBUG | Описание |
|-----------------|--------------|----------|
| `debug`         | 1 (вкл)      | Development-сборка с serial shell, отладка через COM1 |
| `release`       | 0 (выкл)     | Optimized-сборка без serial shell, только framebuffer |
| `secure`        | 0 (выкл)     | Release без COM1-ввода (для production-использования) |

**Важно:** serial shell — привилегированный интерфейс без аутентификации.
Не включайте `SERIAL_DEBUG=1` на реальном железе при физически доступном COM-порте.

В этой конфигурации COM1 остаётся каналом логов, но команды из него не читаются
и framebuffer-консоль в него не зеркалируется.

---

## 🏗️ Архитектура

```
Workspace:
├── kernel/     (vibra-kernel) — lib crate, ядро ОС
│   ├── memory/     — PMM, heap, paging, VMM
│   ├── interrupts/ — IDT, PIC, APIC
│   ├── gdt.rs      — GDT + TSS
│   ├── task/       — preemptive scheduler
│   ├── syscall/    — SYSCALL/SYSRET
│   ├── fs/         — RamFS, VFS, procfs
│   ├── drivers/    — PCI, AHCI
│   ├── shell/      — line editor
│   └── commands/   — 40+ shell commands
└── vibra/      (vibra) — bin crate, точка входа
    └── main.rs     — _start → kernel::boot()
```

Можно собирать как ядро отдельно (`cargo build -p vibra-kernel`), так и ОС вместе с ядром (`cargo build -p vibra`).

**Технологии:**
- Ядро: Rust (no_std, nightly)
- Загрузчик: [Limine](https://github.com/limine-bootloader/limine)
- Эмуляция: QEMU
- UEFI BIOS: OVMF

---

## 📖 Документация

Дополнительная документация находится в папке `docs/`:
- `docs/README.md` — общая информация
- `docs/architecture.md` — архитектура системы
- `docs/roadmap.md` — план развития

---

## 🤝 Участие в проекте

Проект открыт для предложений и Pull Requests:

1. Форкните репозиторий
2. Создайте ветку (`git checkout -b feature/amazing-feature`)
3. Закоммитьте изменения (`git commit -m 'Add amazing feature'`)
4. Запушьте (`git push origin feature/amazing-feature`)
5. Откройте Pull Request

---

## 📜 Лицензия

Проект распространяется под лицензией **MIT**. См. файл [LICENSE](LICENSE).

---

<p align="center">
  <b>Сделано с ❤️ и ☕</b><br/>
  <i>Vibra OS © 2026 OneX01</i>
</p>
