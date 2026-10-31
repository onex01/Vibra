# ext2/ext3/ext4 — чтение

Драйвер `kernel/src/fs/ext2.rs` исторически назывался ext2, но в
`0.2.0-rc.5` читает все три поколения. Запись не поддерживается.

## Что поддерживается

| Возможность | ext2 | ext3 | ext4 |
|---|---|---|---|
| Классические indirect-блоки | да | да | да |
| Extent-дерево (`incompat 0x40`) | — | — | да |
| 64-битные group descriptors (`0x80`) | — | — | да |
| `flex_bg` | — | — | да |
| `dir_index` (htree) | — | — | да, линейным обходом |
| `metadata_csum` | — | — | да, без проверки CRC |
| `large_dir`, `extra_isize` | — | — | да |
| Sparse-файлы (дыры) | да | да | да |
| Быстрые symlink’и (< 60 байт) | да | да | да |
| Запись | нет | нет | нет |

## Что сознательно не поддерживается

`inline_data`, `encrypt`, `bigalloc`, `meta_bg`, `compression`, `journal_dev`
возвращают `FsError::NotSupported` при монтировании. Это лучше, чем
«успешный» mount с пустым каталогом: раньше ext4 с extent-деревом читался как
мусор, потому что `i_block[0..12]` интерпретировались как номера блоков.

## Монтирование

```text
mount /mnt ext4 sda2
mount /mnt ext4 sda2          # alias: ext/ext2/ext3/ext4
mount /mnt auto sda2          # тип определяется по сигнатуре
mount /dev/sda2 /mnt          # Linux-подобная форма
```

Все варианты — **read-only**. Явный `rw` возвращает понятную ошибку:

```text
FAILED: ext2/3/4 driver is read-only (write is not implemented)
```

## Диагностика

- `mount` печатает таблицу блочных устройств с определённым типом ФС;
- `vktst run disk` показывает разделы и их `fs_type`;
- `vktst run fs` проверяет RamFS/procfs, но не ext-тома;
- `make fs-test` создаёт `ext2` (`mke2fs -t ext2 -b 1024`) и `ext4`
  (`mke2fs -t ext4 -b 4096`, дефолтные extents/64bit/metadata_csum) и
  проверяет `ls`/`cat`/вложенные каталоги.

## Ограничения

- Файл читается целиком в память; предел — 64 МиБ
  (`MAX_FILE_BYTES`). Для `cat` больших файлов используйте `diag`/хостовые
  утилиты.
- `dir_index` обходится линейно: htree-узлы пропускаются как `inode = 0`,
  поэтому очень большие каталоги читаются медленнее, но корректно.
- CRC `metadata_csum` не проверяется: драйвер только читает.
- Запись, journal replay и создание файлов не реализованы. Логи пишутся на
  FAT32-раздел `VIBRALOG`, а не на ext4.
