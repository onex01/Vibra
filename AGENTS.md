# AGENTS.md — правила для работы с Vibra

Краткая версия; подробности — в `.opencode/skills/vibra-*`.

## Что это

Экспериментальное монолитное `no_std`-ядро на Rust для `x86_64-unknown-none`,
загрузка Limine/UEFI, основной стенд — QEMU. Не ОС и не дистрибутив.

## Обязательные правила

- Только `core` + `alloc`; `std` запрещён.
- Не печатать из прерываний: `println!` берёт spin-локи.
- Не держать спин-лок через `hlt`, блокирующий диск или ввод.
- Длинные команды обязаны вызывать `crate::poll_cancel()`.
- Запись ФС: только FAT32 и только с явным `rw`. ext2/3/4 и NTFS — read-only.
- Версия меняется только в `kernel/src/version.rs` и `kernel/Cargo.toml`.
- Единицы block layer: `read(sector)` — 512 байт, `read_lba` — блоки
  устройства; смещения разделов — 512 байт.
- Не заявлять поддержку железа по QEMU. SMP не включён.
- Куча растёт по требованию (8 МиБ старт, 4 МиБ шаг, лимит 64 МиБ).
- Не читать файлы целиком: `cat`/`cp`/`mv` работают потоково.

## Проверки перед коммитом

```sh
make check
./build.sh smoke
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
```

## Полезное

```sh
./build.sh doctor      # окружение
./build.sh setup       # Limine + OVMF + hdd.img
make iso-debug         # debug ISO: ESP / VIBRA / VIBRALOG
make usb-debug         # USB debug-образ с тремя разделами
```

Документация: `docs/ext4.md`, `docs/vktst.md`, `docs/release.md`,
`docs/testing.md`, `CHANGELOG.md`.
