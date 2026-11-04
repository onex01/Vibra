# Vibra Kernel — статус проекта

**Версия:** `0.2.0-rc.13` «Foundation RC13»
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

| Подсистема | Состояние 0.2.0-rc.13 |
|---|---|
| Limine, serial, framebuffer | базовый x86_64 boot path |
| PMM, heap, paging/VMM | экспериментально, требуется стресс на железе |
| PS/2, PIC/PIT, GDT/IDT | базовый диагностический путь |
| PCI/ACPI | инвентаризация и частичная инициализация |
| AHCI/IDE/NVMe/ATAPI, FAT32/ext2/3/4/NTFS/exFAT | AHCI + IDE + NVMe + ATAPI CD + block layer + MBR/EBR/GPT + mount/umount/sync; FAT32 rw для `VIBRALOG`, ext/NTFS/exFAT ro (ext4: extents/64bit) в QEMU |
| xHCI/HID | enumeration и HID boot keyboard в QEMU; на реальном железе не подтверждено |
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
| `make clock-test` | `date`/`uptime`/`diag timer`, `/proc/uptime` и состав рамфс |
| `make ramfs-test` | корневой RamFS через VFS: `ls`, запись, `hostname`, `/var/log/boot.log` |
| `make ro-mount-test` | FAT32 read-only по умолчанию; запись только с `rw` |
| `make bad-fs-test` | повреждённые ФС не роняют ядро |
| `make fs-test` | FAT32/ext2/ext4/NTFS read-only + MBR-раздел через shell по serial |
| `make big-fs-test` | NTFS 2 ТБ, GPT выше 2 ТБ, MBR на 4Kn, exFAT по метке |
| `make boot-log-test` | запись `/VIBRA.LOG` на `VIBRALOG` и инкрементальный flush |
| `make debug-image-test` | три раздела, лог начинается с `[BOOT]` |
| `make vktst-test` | `vktst run all`, `free`, `top -b`, `tree` из `/proc` |
| `make debug-image-test` | 3 раздела USB debug-образа и запись `/VIBRA.LOG` |
| `make iso-debug-test` | debug ISO, записанный на USB: разделы и лог |
| `make keyboard-test` | ввод PS/2 и USB HID клавиатуры через monitor `sendkey` |
| `make iso-smoke` | загрузка гибридного ISO в UEFI (OVMF) и BIOS (SeaBIOS) |
| `BOOT_ITERATIONS=10 python3 scripts/qemu-boot-matrix.py` | повторяемость загрузки |
| `python3 scripts/check-warning-budget.py` | бюджет предупреждений компилятора |

Для ручной диагностики дисков есть `disks` и `disks -v`: имя, вид, размер,
смещение и определённый тип ФС.

`make fs-test` (`scripts/qemu-fs-test.sh`) сам создаёт тестовые образы
(`mkfs.fat`, `mke2fs -t ext2`, `mke2fs -t ext4`, `mkntfs`, `sfdisk`) и
прикладывает их к QEMU как `sdb`…`sdf`; результат пишется в
`build/fs-test-status.txt`.

## CI

`.github/workflows/kernel.yml` запускается на `push`, `pull_request` и вручную.
Тулчейн ставится официальным rustup (`nightly`, компонент `rust-src`, target
`x86_64-unknown-none`) в локальные `CARGO_HOME`/`RUSTUP_HOME`, а PATH для
последующих шагов передаётся через `$GITHUB_PATH` — `source $HOME/.cargo/env`
между шагами не действует. Пакеты `rustup`/`rust`/`cargo` из apt или dnf для
сборки не подходят.

В CI выполняются `doctor`, `setup`, бюджеты предупреждений, `smoke`,
`clock-test`, `ramfs-test`, `ro-mount-test`, `bad-fs-test`, `keyboard-test`,
`fs-test`, `vktst-test`, сборка и загрузка debug-образов с тремя разделами,
сборка ISO и его загрузка в UEFI/BIOS, сборка GPT/ESP-образа и его загрузка в
QEMU/OVMF, panic/page-fault отчёты и матрица последовательных загрузок
(10 по умолчанию, `workflow_dispatch` → `boot_iterations`). Логи QEMU
выкладываются артефактом `qemu-logs` даже при падении.

Релиз собирается `.github/workflows/release.yml` по тегу `v*`: release/debug
ISO, UEFI/USB образы, `SHA256SUMS` и GitHub Release. Подробности —
в [release.md](release.md).

## Документы

- `architecture.md` — фактическая архитектура и ограничения.
- `commands.md` — справочник команд shell и горячих клавиш.
- `ext4.md` — чтение ext2/3/4: extents, 64bit, ограничения.
- `vktst.md` — Vibra Kernel Tests: группы, режимы, отчёт.
- `release.md` — версии, имена образов, release workflow.
- `framebuffer.md` — консоль, прокрутка и стоимость отрисовки, debug-only COM1.
- `storage_and_power.md` — ACPI poweroff, каталоги FAT32, приёмники логов.
- `memory.md` — модель памяти и известные риски.
- `testing.md` — матрица QEMU-проверок и критерии приёмки.
- `verified-hardware.md` — шаблон отчёта о прогоне на реальной машине.
- `usb_and_boot.md` — QEMU/UEFI-процедуры и статус USB.
- `roadmap.md` — измеримые критерии версий.
- `usb_progress.md` — инженерные заметки о незавершённом xHCI.

## Версионирование

Единственная публичная версия — версия ядра. До `0.5.0` minor-версии отражают
достигнутые проверяемые hardware milestones, а не количество файлов или
демонстраций. `0.2.0-rc.4` — prerelease Foundation; финальный `0.2.0` ждёт
выполнения CI, QEMU boot matrix и проверки на реальном x86_64.
