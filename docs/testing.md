# Тесты и критерии приёмки

Все проверки запускаются из корня репозитория. QEMU-тесты используют
`build/hdd.img` и `OVMF.fd`, поэтому сначала нужны `./build.sh setup` и
`make build` (или `./build.sh build`).

## Быстрый набор

| Команда | Что проверяет | Обязательные пакеты |
|---|---|---|
| `make check` | debug + no-default сборки и бюджет предупреждений | rustup/nightly, python3 |
| `make smoke` | UEFI boot, маркеры boot-пути, prompt, heartbeat | qemu, mtools |
| `make clock-test` | `date`, `uptime`, `diag timer`, `/proc/uptime`, `ls /` | qemu, mtools |
| `make ramfs-test` | корневой RamFS: `ls`, запись, `hostname`, `/var/log/boot.log`, heap stress | qemu, mtools |
| `make ro-mount-test` | FAT32 read-only по умолчанию, запись только с `rw` | qemu, mtools, dosfstools |
| `make bad-fs-test` | битые FAT32/ext2/NTFS/MBR не роняют ядро | qemu, python3 |
| `make nvme-test` | NVMe Identify/queues, 32K-кластеры, файл 200K, `diag heap-check` | qemu, mtools |
| `make iso-test` | ISO9660 с CD-ROM (ATAPI), Joliet/Rock Ridge, подкаталог | qemu, genisoimage |
| `make big-fs-test` | NTFS 2 ТБ, GPT-раздел за 2 ТБ, MBR на 4Kn, exFAT по метке, FAT16-диагностика | qemu, ntfs-3g, mtools, python3 |
| `make shell-test` | Tab, история, `help`, `diag timer`, отсутствие битого prompt | qemu, mtools |
| `make fs-test` | FAT32/ext2/ext4/NTFS read-only и MBR-раздел | qemu, mtools, e2fsprogs, ntfs-3g, util-linux |
| `make vktst-test` | `vktst run all`, `free`, `top -b`, `tree` из `/proc` | qemu, mtools |
| `make debug-image-test` | три раздела USB debug-образа и запись `/VIBRA.LOG` | qemu, mtools, util-linux |
| `make iso-debug-test` | debug ISO на USB: разделы и запись лога | qemu, mtools, util-linux, xorriso |
| `make keyboard-test` | PS/2 и USB HID через monitor `sendkey` | qemu, python3 |
| `make uhci-test` | USB-клавиатура на UHCI (как на чипсетах Intel/AMD) | qemu |
| `make iso-smoke` | загрузка гибридного ISO в UEFI и BIOS | qemu, xorriso, OVMF, SeaBIOS |
| `BOOT_ITERATIONS=10 python3 scripts/qemu-boot-matrix.py` | повторяемость загрузки | qemu, python3 |

Полный прогон перед релизом:

```sh
./build.sh doctor
./build.sh setup
make check
./build.sh smoke
./build.sh clock-test
./build.sh ramfs-test
./build.sh ro-mount-test
./build.sh bad-fs-test
./build.sh nvme-test
./build.sh big-fs-test
./build.sh iso-test
./build.sh shell-test
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
./build.sh iso-debug-test
./build.sh keyboard-test
./build.sh iso
make iso-smoke
./build.sh uefi-usb
BOOT_ITERATIONS=100 python3 scripts/qemu-boot-matrix.py
```

На CI без KVM QEMU работает через TCG, поэтому таймауты заданы с запасом:
`SMOKE_TIMEOUT`, `BOOT_TIMEOUT`, `RAMFS_TEST_TIMEOUT`, `RO_MOUNT_TEST_TIMEOUT`,
`BAD_FS_TEST_TIMEOUT`, `NVME_TEST_TIMEOUT`, `BIG_FS_TEST_TIMEOUT`

## exFAT-фикстура

`mkfs.exfat` умеет только форматировать том, а положить в него файлы без
root и loop-устройства нечем. Поэтому `scripts/mk-exfat-fixture.py` собирает
образ вручную: boot sector с checksum, FAT, cluster heap, entry set
`0x85`+`0xC0`+`0xC1`, метка `0x83`. В образе есть contiguous-файл,
подкаталог и файл из двух кластеров через FAT — так проверяется и разбор
каталога, и порядок цепочки., `SHELL_TEST_TIMEOUT`, `FS_TEST_TIMEOUT`, `KEYBOARD_TEST_TIMEOUT`.

## Что считается регрессией

- `ls /` теряет каталоги `dev proc sys cfg home var tmp mnt`;
- `touch`/`cp`/`hostname` сообщают успех, но данные не читаются обратно;
- `/var/log/boot.log` пустой или без `[BOOT]`-маркеров;
- монтирование FAT32 без `rw` пропускает запись;
- повреждённый образ ФС приводит к `KERNEL PANIC` или `[FATAL]`;
- `diag fault-test` не печатает CR2/error code/RIP;
- `diag heap-test` показывает утечку;
- `diag heap-check` находит битый заголовок free-list;
- `diag heap-big 12` не может вырастить кучу (`heap-big: FAILED`);
- `cat` файла больше 2 МиБ падает в OOM вместо потокового вывода;
- `vktst run all` показывает `failed`;
- debug-образ не находит `VIBRALOG` или `/VIBRA.LOG` пуст;
- `tree` без аргумента начинает с `/` вместо текущего каталога.

## Ручная проверка на реальном железе

Перед запуском на ноутбуке:

1. Записать `build/vibra-uefi.img` на **отдельный** тестовый USB-носитель.
2. Отключить Secure Boot.
3. Проверить `./build.sh doctor` на хосте.
4. После загрузки выполнить `diag hw`, `diag timer`, `diag keyboard`,
   `date`, `uptime`, `ls /`, `mount` и сохранить лог (`log`, `/var/log/boot.log`).

Результаты и модель машины/прошивки стоит фиксировать в отдельном файле
`docs/verified-hardware.md` (шаблон можно добавить при первом прогоне).