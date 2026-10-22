# Vibra Kernel — статус проекта

**Версия:** `0.2.0-rc.1` «Foundation RC1»
**Поддерживаемая архитектура:** только `x86_64-unknown-none`  
**Загрузка:** Limine через UEFI; QEMU — основная тестовая среда

## Назначение

Vibra Kernel — экспериментальное ядро на Rust `no_std`, а не операционная
система. Приоритет — воспроизводимый запуск и диагностика на реальном x86_64
оборудовании. Пользовательская среда, GUI, приложения и ARM64 исключены из
текущего scope.

## Подтверждённое и экспериментальное

Подтверждено только прохождение компиляции и QEMU-путь, когда он явно
зафиксирован тестом. Наличие исходного файла не является поддержкой железа.

| Подсистема | Состояние 0.2.0-rc.1 |
|---|---|
| Limine, serial, framebuffer | базовый x86_64 boot path |
| PMM, heap, paging/VMM | экспериментально, требуется стресс на железе |
| PS/2, PIC/PIT, GDT/IDT | базовый диагностический путь |
| PCI/ACPI | инвентаризация и частичная инициализация |
| AHCI/NVMe, FAT32/Ext2/NTFS | AHCI + block layer + MBR/GPT + read-only mount в QEMU |
| xHCI/HID | только диагностика контроллера, не ввод с USB |
| e1000/RTL8139, IPv4/ICMP | QEMU-ориентированный прототип, без DHCP/TCP |
| ARM64 / Orange Pi Zero 3 | не поддерживается |

## Границы репозитория

`kernel/` — workspace crate ядра и его точка входа (`kernel/src/main.rs`).
`vibra/` — legacy-исходники, не входят в текущую workspace-сборку. Каталоги
`apps/` и GUI-код также считаются legacy-экспериментами и не входят в критерии
релиза 0.1–0.5.

## Как воспроизвести проверки

| Команда | Что проверяет |
|---|---|
| `make build` | сборка ядра и установка его в `build/hdd.img` |
| `make smoke` | UEFI-загрузка в QEMU, маркеры boot-пути и heartbeat |
| `make fs-test` | FAT32/ext2/NTFS read-only + MBR-раздел через shell по serial |
| `BOOT_ITERATIONS=10 python3 scripts/qemu-boot-matrix.py` | повторяемость загрузки |
| `python3 scripts/check-warning-budget.py` | бюджет предупреждений компилятора |

`make fs-test` (`scripts/qemu-fs-test.sh`) сам создаёт тестовые образы
(`mkfs.fat`, `mke2fs`, `mkntfs`, `sfdisk`) и прикладывает их к QEMU как
`sdb`…`sde`; результат пишется в `build/fs-test-status.txt`.

## Документы

- `architecture.md` — фактическая архитектура и ограничения.
- `memory.md` — модель памяти и известные риски.
- `usb_and_boot.md` — QEMU/UEFI-процедуры и статус USB.
- `roadmap.md` — измеримые критерии версий.
- `usb_progress.md` — инженерные заметки о незавершённом xHCI.

## Версионирование

Единственная публичная версия — версия ядра. До `0.5.0` minor-версии отражают
достигнутые проверяемые hardware milestones, а не количество файлов или
демонстраций. `0.2.0-rc.1` — prerelease Foundation; финальный `0.2.0` ждёт
выполнения CI, QEMU boot matrix и проверки на реальном x86_64.
