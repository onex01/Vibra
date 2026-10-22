<h1 align="center">🌊 Vibra Kernel</h1>

<p align="center"><b>Экспериментальное x86_64-ядро на Rust</b></p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.2.0--rc.1--Foundation-blue?style=flat-square" alt="Version"/>
  <img src="https://img.shields.io/badge/target-x86__64-orange?style=flat-square" alt="Target"/>
  <img src="https://img.shields.io/badge/Rust-no__std-red?style=flat-square" alt="Rust"/>
  <img src="https://img.shields.io/badge/license-MIT-yellow?style=flat-square" alt="License"/>
</p>

## Статус проекта

Vibra — **не операционная система и не готовый дистрибутив**. Это раннее
монолитное ядро для x86_64, разрабатываемое для освоения bare-metal Rust и
достижения устойчивой работы на реальном оборудовании. Версия `0.2.0-rc.1`
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
- прототипы AHCI, NVMe, e1000/RTL8139, xHCI и файловых систем;
- RAM-файловая система и встроенная shell для диагностики.

Это не подтверждает работу соответствующих драйверов на реальном железе.
В частности, USB HID, автомонтирование дисков, DHCP/TCP и ARM64 не
реализованы. Подробности и критерии готовности — в [roadmap](docs/roadmap.md).

## Быстрый старт: QEMU

```sh
./build.sh doctor
./build.sh setup
./build.sh smoke
./build.sh uefi-usb
./build.sh debug
```

`smoke` запускает QEMU на 75 секунд, проверяет serial boot log до prompt
`vibra:/>` и требует не менее 12 heartbeat-сообщений через отдельный debugcon.
Serial остаётся доступен для ввода команд; логи сохраняются в `build/`.

`debug` включает serial shell. Не используйте его на открытом физическом
serial-порту. Для образа без serial-ввода используйте `./build.sh secure`.
`uefi-usb` создаёт GPT-образ с FAT32 EFI System Partition. Для bundled Limine
отключите Secure Boot. Команда создаёт только файл образа и ничего не пишет на
физический носитель.

### Debian/Ubuntu

```sh
sudo apt update
sudo apt install make qemu-system-x86 mtools dosfstools wget tar xorriso fdisk
rustup toolchain install nightly
rustup target add x86_64-unknown-none --toolchain nightly
```

### Fedora, RHEL, Rocky, AlmaLinux

```sh
sudo dnf install make qemu-system-x86 mtools dosfstools wget tar xorriso util-linux
rustup toolchain install nightly
rustup target add x86_64-unknown-none --toolchain nightly
```

`./build.sh doctor` определяет семейство дистрибутива и подсказывает точную
команду установки отсутствующих утилит. Скрипт ничего не устанавливает сам.

## Команды сборки

```text
./build.sh doctor         проверить окружение
./build.sh setup          скачать Limine/OVMF и создать QEMU-образ
./build.sh build          собрать debug kernel.elf
./build.sh smoke          проверить UEFI boot и shell prompt в QEMU
./build.sh debug          собрать и запустить QEMU
./build.sh secure         QEMU без serial shell
./build.sh iso            собрать экспериментальный ISO
./build.sh usb            собрать образ USB; запись на устройство выполняется вручную
```

Образ USB — экспериментальный. Не записывайте его на единственный носитель с
данными и не тестируйте операции записи драйверов на ценных дисках.

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
- [USB и загрузка](docs/usb_and_boot.md)
- [Дорожная карта](docs/roadmap.md)

## Лицензия

MIT. См. [LICENSE](LICENSE).
