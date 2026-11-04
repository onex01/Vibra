# Сборка и релиз

## Версия

Единственный источник версии — `kernel/src/version.rs`:

```rust
pub const KERNEL_VERSION: &str = "0.2.0-rc.10";
pub const KERNEL_CODENAME: &str = "Foundation RC10";
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
clock/ramfs/ro-mount/bad-fs/nvme/shell/fs, vktst, debug-образы, ISO и матрица
загрузок.

`.github/workflows/release.yml` — релиз:

- запускается по тегу `v*` или вручную (`workflow_dispatch` → `tag`);
- ставит пакеты и nightly-тулчейн;
- выполняет `doctor`, `setup`, `check`, `build`, `smoke`, `fs-test`;
- собирает `iso-release`, `iso-debug`, `uefi-usb`, `usb`, `usb-debug`;
- проверяет debug-образы загрузкой в QEMU;
- кладёт артефакты и `SHA256SUMS` в `dist/`;
- публикует GitHub Release (`gh release create`) с автоматическими notes.

### Выпуск версии

```sh
# 1. обновить kernel/src/version.rs, CHANGELOG.md, README/docs
make check
./build.sh smoke
./build.sh fs-test
git add -A
git commit -m "release: 0.2.0-rc.10"
git tag v0.2.0-rc.10
git push origin main --tags
```

Тег запускает `release.yml`; артефакты появятся в GitHub Releases.
