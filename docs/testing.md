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
| `make nvme-test` | NVMe Identify/queues, block layer, `mount auto`, 4Kn | qemu, mtools |
| `make shell-test` | Tab, история, `help`, `test`, отсутствие битого prompt | qemu, mtools |
| `make fs-test` | FAT32/ext2/NTFS read-only и MBR-раздел | qemu, mtools, e2fsprogs, ntfs-3g, util-linux |
| `make keyboard-test` | PS/2 и USB HID через monitor `sendkey` | qemu, python3 |
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
./build.sh shell-test
./build.sh fs-test
./build.sh keyboard-test
./build.sh iso
make iso-smoke
./build.sh uefi-usb
BOOT_ITERATIONS=100 python3 scripts/qemu-boot-matrix.py
```

На CI без KVM QEMU работает через TCG, поэтому таймауты заданы с запасом:
`SMOKE_TIMEOUT`, `BOOT_TIMEOUT`, `RAMFS_TEST_TIMEOUT`, `RO_MOUNT_TEST_TIMEOUT`,
`BAD_FS_TEST_TIMEOUT`, `NVME_TEST_TIMEOUT`, `SHELL_TEST_TIMEOUT`, `FS_TEST_TIMEOUT`, `KEYBOARD_TEST_TIMEOUT`.

## Что считается регрессией

- `ls /` теряет каталоги `dev proc sys cfg home var tmp mnt`;
- `touch`/`cp`/`hostname` сообщают успех, но данные не читаются обратно;
- `/var/log/boot.log` пустой или без `[BOOT]`-маркеров;
- монтирование FAT32 без `rw` пропускает запись;
- повреждённый образ ФС приводит к `KERNEL PANIC` или `[FATAL]`;
- `diag fault-test` не печатает CR2/error code/RIP;
- `diag heap-test` показывает утечку.

## Ручная проверка на реальном железе

Перед запуском на ноутбуке:

1. Записать `build/vibra-uefi.img` на **отдельный** тестовый USB-носитель.
2. Отключить Secure Boot.
3. Проверить `./build.sh doctor` на хосте.
4. После загрузки выполнить `diag hw`, `diag timer`, `diag keyboard`,
   `date`, `uptime`, `ls /`, `mount` и сохранить лог (`log`, `/var/log/boot.log`).

Результаты и модель машины/прошивки стоит фиксировать в отдельном файле
`docs/verified-hardware.md` (шаблон можно добавить при первом прогоне).