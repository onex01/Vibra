# Сборка и релиз

## Версия

Единственный источник версии — `kernel/src/version.rs`:

```rust
pub const KERNEL_VERSION: &str = "0.2.0-rc.25.4";
pub const KERNEL_CODENAME: &str = "Foundation RC25.4";
```

`Makefile` читает `KERNEL_VERSION` из этого файла, `kernel/build.rs` добавляет
`BUILD_DATE`, `GIT_COMMIT` и `BUILD_PROFILE`. `version` и `uname -a` показывают
эти значения.

Дату сборки можно зафиксировать для воспроизводимости:

```sh
SOURCE_DATE_EPOCH=1793491200 make iso-release
```

## Имена артефактов

`BUILD_DATE` по умолчанию — `date -u +%Y-%m-%d`. Переопределяется
`make BUILD_DATE=2026-10-31 ...`.

| Цель | Артефакт |
|---|---|
| `make iso` | `build/vibra_<DATE>_<VERSION>.iso` |
| `make iso-release` | `build/vibra-release_<DATE>_<VERSION>.iso` |
| `make iso-debug` | `build/vibra-debug_<DATE>_<VERSION>.iso` |
| `make uefi-usb` | `build/vibra-uefi_<DATE>_<VERSION>.img` |
| `make usb` | `build/vibra-usb_<DATE>_<VERSION>.img` |
| `make usb-debug` | `build/vibra-usb-debug_<DATE>_<VERSION>.img` |

Для совместимости со скриптами и документацией создаются симлинки без даты:
`build/vibra.iso`, `build/vibra-debug.iso`, `build/vibra-uefi.img`,
`build/vibra-usb.img`, `build/vibra-usb-debug.img`.

## Debug-образ: три раздела

`make usb-debug` создаёт GPT-образ:

1. `VIBRA-ESP` — FAT32, `EFI/BOOT/BOOTX64.EFI` + `limine/limine.conf`;
2. `VIBRA` — FAT32, `kernel.elf`; Limine грузит его как
   `fslabel(VIBRA):/kernel.elf`;
3. `VIBRALOG` — FAT32 с предвыделенным `/VIBRA.LOG`.

`make iso-debug` добавляет к ISO те же `VIBRA` и `VIBRALOG` через
`xorriso -append_partition`. ISO, записанный на USB, видит все разделы через
AHCI/ATAPI и пишет лог; CD/DVD физически только для чтения, и ядро сообщает
об этом:

```text
[LOG] VIBRALOG is on read-only media (CD/DVD); flash the ISO to USB to persist logs
```

## Локальный полный прогон

```sh
./build.sh doctor
./build.sh setup
make check
./build.sh smoke
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
./build.sh iso-debug-test
./build.sh iso-release
```

## GitHub Actions

`.github/workflows/kernel.yml` — обычный CI на push/PR: сборка, smoke,
clock/ramfs/ro-mount/bad-fs/nvme/big-fs/iso/shell/fs, vktst, debug-образы,
ISO и матрица загрузок.

`.github/workflows/release.yml` — авторелиз. Запускается в трёх случаях:

| Триггер | Поведение |
|---|---|
| push в `main` | релиз собирается сам, **если** версия в `kernel/src/version.rs` ещё не имеет тега `v<версия>`; `[skip release]` в сообщении коммита отменяет |
| push тега `v*` | пересборка и перезаливка существующего релиза |
| `workflow_dispatch` | вручную; можно задать тег и `force` (пересобрать, даже если релиз есть) |

Job'ы:

1. **prepare** — читает `KERNEL_VERSION`/`KERNEL_CODENAME`, вычисляет тег
   (`v<версия>`), решает `should_release`, ставит `prerelease` для `-rc.*`;
2. **build** — packages + nightly, `doctor`, `setup`, бюджеты предупреждений,
   `build`, `smoke`, `fs-test`, затем `iso-release`, `iso-debug`,
   `uefi-usb`, `usb`, `usb-debug`; проверяет debug-образы загрузкой в QEMU;
   собирает `dist/` (только файлы текущей версии), `SHA256SUMS`,
   `MANIFEST.txt`, `RELEASE_NOTES.md`;
3. **publish** — создаёт тег, если его ещё нет, и публикует релиз с notes из
   `scripts/release-notes.sh`.

Описание релиза генерируется, а не пишется руками: раздел под текущую версию
из `CHANGELOG.md`, список коммитов с предыдущего тега, таблица образов с
размерами и контрольными суммами, известные ограничения.

```sh
# то же самое локально, без GitHub
make dist              # все образы + dist/{SHA256SUMS,MANIFEST.txt,RELEASE_NOTES.md}
make release-notes     # только текст описания для build/dist
```

### С serial или без

Debug-образ существует в двух вариантах, разница — только в наличии канала
COM1:

| Образ | Когда брать |
|---|---|
| `vibra-debug_*` | есть COM-порт и нужен полный serial-лог (или QEMU с `-serial`) |
| `vibra-debug-noserial_*` | COM-порта нет **или** важна скорость: каждый символ в UART при 115200 занимает 87 мкс, и на длинном выводе это заметно |
| `vibra-release_*` | нужна только скорость, раздел логов не нужен |

Boot-лог в обоих debug-вариантах пишется в `/VIBRA.LOG` на разделе
`VIBRALOG`, поэтому `noserial` не теряет диагностику — только мгновенный
поток в терминал.

```sh
./build.sh iso-debug-noserial   # build/vibra-debug-noserial_<DATE>_<VERSION>.iso
./build.sh usb-debug-noserial   # build/vibra-usb-debug-noserial_<DATE>_<VERSION>.img
./build.sh debug-variants       # все четыре debug-образа
NOSERIAL=1 bash scripts/qemu-debug-image-test.sh <образ>   # проверка по /VIBRA.LOG
```

То же есть в меню: `./build.sh` → `3) Build images`.

### Выпуск версии

```sh
# 1. обновить kernel/src/version.rs, CHANGELOG.md, README/docs
make check
./build.sh smoke
./build.sh fs-test
git add -A
git commit -m "release: 0.2.0-rc.25.4"
git push origin main      # этого достаточно: тег и релиз создаст CI
```

Тег можно поставить и руками (`git tag v0.2.0-rc.25.4 && git push --tags`) —
тогда `release.yml` пересоберёт и перезальёт релиз, а не создаст дубликат.

### Что попадает в релиз

| Файл | Назначение |
|---|---|
| `vibra-release_<DATE>_<VERSION>.iso` | релизный ISO (UEFI+BIOS), UART выключен |
| `vibra-debug_<DATE>_<VERSION>.iso` | отладочный ISO: serial-лог, разделы `VIBRA` и `VIBRALOG` |
| `vibra-usb_<DATE>_<VERSION>.img` | образ для флешки, GPT+ESP |
| `vibra-usb-debug_<DATE>_<VERSION>.img` | флешка с тремя разделами для разбора проблем на железе |
| `vibra-debug-noserial_<DATE>_<VERSION>.iso` | то же без COM1: скорость release, лог на `VIBRALOG` |
| `vibra-usb-debug-noserial_<DATE>_<VERSION>.img` | то же на флешке |
| `vibra-uefi_<DATE>_<VERSION>.img` | минимальный GPT/ESP-образ |
| `SHA256SUMS` | контрольные суммы всех файлов |
| `MANIFEST.txt` | версия, коммит, дата, тулчейн, SHA256 `OVMF.fd` |
| `RELEASE_NOTES.md`, `CHANGELOG.md` | описание релиза и полная история |
