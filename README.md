<h1 align="center">🌊 Vibra Kernel</h1>

<p align="center"><b>Экспериментальное x86_64-ядро на Rust</b></p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.2.0--rc.25.2--Foundation-blue?style=flat-square" alt="Version"/>
  <img src="https://img.shields.io/badge/target-x86__64-orange?style=flat-square" alt="Target"/>
  <img src="https://img.shields.io/badge/Rust-no__std-red?style=flat-square" alt="Rust"/>
  <img src="https://img.shields.io/badge/license-MIT-yellow?style=flat-square" alt="License"/>
</p>

## Статус проекта

Vibra — **не операционная система и не готовый дистрибутив**. Это раннее
монолитное ядро для x86_64, разрабатываемое для освоения bare-metal Rust и
достижения устойчивой работы на реальном оборудовании. Версия `0.2.0-rc.25.2`
обозначает prerelease этапа Foundation; API, структура репозитория и форматы образов могут
меняться без обратной совместимости.

Текущая цель — надёжный запуск с UEFI/Limine, диагностика через serial и
framebuffer, безопасная память, прерывания, PCI, блочное I/O и базовые
устройства. Desktop, приложения, скриптовый язык и пользовательская среда не
являются целями текущего релиза. ARM64 будет начат только после выполнения
критериев `0.5.0` на реальном x86_64-железе.

Во время загрузки serial-лог также отображается на Limine framebuffer и
остаётся на экране после перехода в shell; отдельный ASCII-баннер отключён.

## Что действительно есть

- загрузка x86_64 через Limine в QEMU;
- serial COM1, framebuffer-консоль, GDT/TSS/IDT, PIC и задел APIC;
- PMM, heap и экспериментальный VMM;
- PS/2-клавиатура, ACPI и PCI-инвентаризация;
- AHCI/NVMe/ATAPI (CD), IDE/PATA, e1000/RTL8139, xHCI и UHCI (USB 1.1);
- FAT32 (rw для `VIBRALOG`), ext2/3/4 (ro, включая extents и 64bit),
  NTFS (ro), exFAT (ro, включая contiguous-файлы и FAT-цепочки) и ISO9660
  на CD/DVD (ro, Joliet/Rock Ridge);
- RAM-файловая система, встроенная shell, `vktst` и `disks` для диагностики.

Это не подтверждает работу соответствующих драйверов на реальном железе.
В частности, OHCI, автомонтирование дисков, DHCP/TCP и ARM64 не реализованы;
HID boot keyboard, UHCI-клавиатура, USB-накопители на xHCI и EHCI работают в
QEMU (`make keyboard-test`, `make uhci-test`, `make usb-storage-test`,
`make ehci-test`), но на физическом железе подтверждены не для всех платформ.
Подробности и критерии готовности — в [roadmap](docs/roadmap.md).

## Быстрый старт

Без аргументов `build.sh` открывает интерактивное меню (на английском, чтобы
не зависеть от локали терминала): сборка, запуск в QEMU, образы, тесты и
диагностика. Полный список команд остаётся доступен напрямую — им пользуются
скрипты и CI.

```sh
./build.sh            # меню
./build.sh list       # все команды
```

## Быстрый старт: QEMU

```sh
./build.sh doctor
./build.sh setup
./build.sh smoke
./build.sh clock-test
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
./build.sh iso-debug-test
./build.sh uefi-usb
./build.sh debug
```

`smoke` запускает QEMU на 90 секунд, проверяет serial boot log до prompt
`vibra:/>` и требует не менее 12 heartbeat-сообщений через отдельный debugcon.
Serial остаётся доступен для ввода команд; логи сохраняются в `build/`.

`clock-test` проверяет монотонные часы и RTC: `date`, `uptime`, `diag timer`,
`/proc/uptime` и отсутствие удалённых каталогов (`/os`, `/apps`, `/lib`) в рамфс.
`debug` включает serial shell. Не используйте его на открытом физическом
serial-порту: для запуска без serial-ввода используйте
`./build.sh run-serial-file` (лог в файл), а для debug-образа с логом на
носителе, но без COM1 — `iso-debug-noserial` или
`usb-debug-noserial`. Каждый символ в COM1 при 115200 стоит ~87 мкс, поэтому
на длинном выводе вариант без serial заметно быстрее, а диагностика остаётся
в `/VIBRA.LOG` на разделе `VIBRALOG`.
`uefi-usb` создаёт GPT-образ с FAT32 EFI System Partition. Для bundled Limine
отключите Secure Boot. Команда создаёт только файл образа и ничего не пишет на
физический носитель.

### Rust: только официальный rustup

Пакеты `rustup`/`rust`/`cargo` из apt или dnf не дают готового рабочего
`rustup` (команда может просто отсутствовать в `PATH`) и не содержат
nightly-компонентов, поэтому тулчейн ставится установщиком:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup toolchain install nightly --component rust-src
rustup target add x86_64-unknown-none --toolchain nightly
```

`rust-src` обязателен: ядро собирается с `-Z build-std=core,alloc,compiler_builtins`
(см. `.cargo/config.toml`), а `build-std` берёт исходники `core`/`alloc` из
компонента `rust-src` для nightly. `source "$HOME/.cargo/env"` действует только
в текущей оболочке — после перезапуска терминала его нужно повторить (или
перелогиниться, если установщик добавил строку в профиль).

### Debian/Ubuntu

```sh
sudo apt update
sudo apt install make qemu-system-x86 mtools dosfstools wget tar xz-utils xorriso fdisk
```

### Fedora, RHEL, Rocky, AlmaLinux

```sh
sudo dnf install make qemu-system-x86 mtools dosfstools wget tar xz-utils xorriso util-linux
```

Для `make fs-test` дополнительно нужен `ntfs-3g` (команды `mkntfs`, `ntfscp`).

`./build.sh doctor` определяет семейство дистрибутива и подсказывает точную
команду установки отсутствующих утилит, nightly, target и компонента `rust-src`.
Скрипт ничего не устанавливает сам.

## Команды сборки

```text
./build.sh doctor         проверить окружение (включая nightly, target, rust-src)
./build.sh setup          скачать Limine/OVMF и создать QEMU-образ
./build.sh build          собрать debug kernel.elf
./build.sh smoke          проверить UEFI boot и shell prompt в QEMU
./build.sh clock-test     проверить RTC, uptime и diag timer
./build.sh ramfs-test     проверить корневой RamFS, запись файлов и /var/log/boot.log
./build.sh ro-mount-test  проверить read-only монтирование FAT32
./build.sh bad-fs-test    проверить устойчивость к повреждённым ФС
./build.sh fs-test        проверить FAT32/ext2/ext4/NTFS, MBR-раздел, sync/umount
./build.sh big-fs-test    проверить NTFS 2 ТБ, GPT выше 2 ТБ, MBR на 4Kn, exFAT
./build.sh iso-test       смонтировать ISO9660 с CD-ROM (sr0) и прочитать файлы
./build.sh noserial-test  загрузка без COM1 (регрессия зависания на Step 4)
./build.sh serial-input-test  ввод через COM1, а не через monitor
./build.sh build-script-test  проверить сам build.sh (таблица, меню, цели)
./build.sh iso-debug      debug ISO: serial-лог + раздел VIBRALOG
./build.sh iso-debug-noserial  то же без COM1: скорость release, лог на месте
./build.sh usb-debug      флешка с тремя разделами (ESP, VIBRA, VIBRALOG)
./build.sh usb-debug-noserial  то же без COM1
./build.sh debug-variants собрать все четыре debug-образа
./build.sh dist           все релизные образы + SHA256SUMS + описание релиза
./build.sh vktst-test     проверить vktst, top, free и tree
./build.sh debug-image-test  проверить 3 раздела USB debug-образа и запись лога
./build.sh iso-debug-test проверить debug ISO, записанный на USB
./build.sh keyboard-test  проверить ввод PS/2 и USB HID
./build.sh uhci-test      проверить USB-клавиатуру на UHCI
./build.sh usb3-test      проверить USB3-диск на xHCI и hotplug
./build.sh usb-storage-test  проверить FAT32 на USB-диске через VFS
./build.sh ehci-test      проверить EHCI (USB 2.0): диск, монтирование, hotplug
./build.sh iso            собрать гибридный BIOS/UEFI ISO
./build.sh iso-smoke      загрузить ISO в UEFI (OVMF) и BIOS (SeaBIOS)
./build.sh uefi-usb       собрать GPT-образ с EFI System Partition
./scripts/hw-probe.sh     снять паспорт железа под Linux (для отладки драйверов)
./build.sh check          сборка debug/release-конфигураций + бюджет предупреждений
./build.sh debug          собрать и запустить QEMU
./build.sh run-serial-file  QEMU с serial-логом в файл (без ввода в stdio)
./build.sh run-serial-tcp   QEMU с serial на TCP-порт
./build.sh usb            собрать образ USB; запись на устройство выполняется вручную
```

Образ USB — экспериментальный. Не записывайте его на единственный носитель с
данными и не тестируйте операции записи драйверов на ценных дисках.

Монтирование дисков по умолчанию **read-only**. Запись включается только явным
аргументом: `mount /mnt fat32 sda1 rw`. ext2/3/4, NTFS, exFAT и ISO9660
остаются read-only:
драйверы не поддерживают запись. Boot-лог пишется на FAT32-раздел `VIBRALOG`.

Имена образов содержат дату и версию, например
`vibra_2026-10-31_0.2.0-rc.25.iso` и
`vibra-debug_2026-10-31_0.2.0-rc.25.iso`; для совместимости создаются
симлинки `build/vibra.iso` и `build/vibra-debug.iso`.

## CI

Workflow `.github/workflows/kernel.yml` запускается на `push`, `pull_request` и
вручную (`workflow_dispatch`, вход `boot_iterations`). Он ставит хост-пакеты
через apt, затем ставит Rust официальным установщиком в локальные
`CARGO_HOME`/`RUSTUP_HOME` (`.ci/` в рабочем каталоге, в git не попадает),
добавляет `$CARGO_HOME/bin` в `$GITHUB_PATH` и устанавливает
`nightly --component rust-src` + target `x86_64-unknown-none`.

Проверки: `doctor`, `setup`, бюджеты предупреждений, `smoke`, `clock-test`,
`keyboard-test`, `fs-test`, `vktst-test`, debug-образы с тремя разделами,
сборка ISO и его загрузка в UEFI/BIOS, сборка GPT/ESP-образа,
panic/page-fault отчёты и последовательная матрица загрузок.
На раннерах нет KVM, поэтому QEMU работает через TCG, а таймауты тестов заданы
с запасом через переменные окружения (`SMOKE_TIMEOUT`, `BOOT_TIMEOUT` и т.д.).
Логи QEMU выкладываются артефактом `qemu-logs` даже при падении.

## Структура

```text
kernel/    workspace crate: библиотека, entry point и подсистемы ядра
vibra/     legacy-исходники, не входят в workspace-сборку
docs/      актуальные ограничения, архитектура и roadmap
```

## Документация

- [Статус и границы проекта](docs/README.md)
- [Архитектура](docs/architecture.md)
- [Память](docs/memory.md)
- [ext2/3/4](docs/ext4.md)
- [vktst](docs/vktst.md)
- [Сборка и релиз](docs/release.md)
- [USB и загрузка](docs/usb_and_boot.md)
- [Тестовые машины](docs/hardware/matrix.md)
- [Дорожная карта](docs/roadmap.md)
- [Changelog](CHANGELOG.md)

## Лицензия

MIT. См. [LICENSE](LICENSE).
