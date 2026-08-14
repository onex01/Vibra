# Vibra OS

**Версия ОС:** 1.1.0 "Sirius"<br>
**Версия ядра:** 1.1.0 "Boson"<br>
**Архитектура:** x86_64 (ARM64 подготовлен заглушками)<br>
**Язык:** Rust (ядро), C (портируемый софт)<br>
**Загрузчик:** Limine<br>
**Файловая система:** VFS + RamFS/procfs/sysfs/devtmpfs, FAT32, Ext2

## Описание

Vibra — это хобби-операционная система, разрабатываемая с нуля на Rust.  
Цель проекта — создать современную учебную ОС с графическим интерфейсом, приложениями, сетевой и дисковой подсистемами, а в перспективе с поддержкой реального железа и ARM64.

## Текущий статус

- [x] Загрузка через Limine (UEFI)
- [x] Вывод в последовательный порт (COM1)
- [x] Физический менеджер памяти (PMM)
- [x] Вывод на экран (framebuffer)
- [x] PS/2 клавиатура с поддержкой Shift
- [x] Графическая консоль с ASCII-арт логотипом
- [x] VFS, RamFS, procfs, sysfs, devtmpfs
- [x] FAT32 read/write и Ext2 read
- [x] Shell с расширенным набором команд
- [x] Собственные GDT/TSS/IDT, PMM, heap, paging, VMM
- [x] Вытесняющий планировщик задач
- [x] Ring 3 и syscall/sysret
- [x] ACPI, PCI, AHCI, NVMe, e1000/RTL8139, VirtIO, базовая USB-детекция
- [x] Графические демо и desktop-команда
- [ ] Полный USB HID (xHCI keyboard/mouse)
- [ ] Полная миграция IRQ на APIC
- [ ] Автомонтирование boot-диска в VFS

## Доступные команды shell

```bash
help             # показать справку
clear            # очистить экран
echo <text>      # вывести текст
quit             # остановить систему
version          # показать версию ОС
version kernel   # показать версию ядра
ls               # список файлов и директорий
touch <name>     # создать пустой файл
mkdir <name>     # создать директорию
cat <file>       # прочитать содержимое файла
rm <name>        # удалить файл или директорию
run <script.vs>  # запустить Vibra Script
desktop          # открыть графический desktop
display          # показать графические бэкенды
heap             # статистика кучи
diag             # диагностика ядра
tasks / ps / top # задачи и планировщик
```

## Структура проекта

```
kernel/                  # vibra-kernel, no_std lib crate
├── src/
│   ├── lib.rs           # boot/init и публичные модули ядра
│   ├── memory/          # PMM, heap, paging, VMM
│   ├── interrupts/      # IDT, PIC, APIC, MSI
│   ├── task/            # вытесняющий планировщик
│   ├── syscall/         # syscall/sysret
│   ├── fs/              # VFS, RamFS, FAT32, Ext2, proc/sys/dev tmpfs
│   ├── drivers/         # PCI, AHCI, NVMe, USB, net, VirtIO GPU
│   └── commands/        # kernel shell commands
vibra/                   # bin crate ОС
├── src/
│   ├── main.rs          # _start -> kernel::init() + команды ОС
│   ├── commands/        # GUI/железные команды ОС
│   └── gui/             # compositor, cursor, widgets
docs/
├── README.md            # этот файл
├── architecture.md      # архитектура системы
├── memory.md            # подсистема памяти
├── vibra_script.md      # язык Vibra Script
└── roadmap.md           # план развития
├── Makefile
├── limine.conf
├── linker.ld
└── Cargo.toml
```

## Версионирование

**Версия ОС** обновляется при добавлении новых модулей и приложений.<br>
**Версия ядра** обновляется при изменениях в ядре (память, драйверы, планировщик).

Пример: `Vibra OS v1.1.0 "Sirius" | Kernel v1.1.0 "Boson"`

## Сборка и запуск

```bash
./build.sh doctor # проверить зависимости
./build.sh setup  # первый запуск: Limine/OVMF + build/hdd.img
./build.sh debug  # сборка и QEMU
make run          # быстрый запуск через Makefile
```

Ubuntu/Debian зависимости:

```bash
sudo apt update
sudo apt install qemu-system-x86 mtools dosfstools wget xorriso fdisk make
rustup toolchain install nightly
rustup target add x86_64-unknown-none --toolchain nightly
```

## Лицензия

MIT License
