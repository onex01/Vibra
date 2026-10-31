# Changelog

## 0.2.0-rc.6 «Foundation RC6» — 2026-11-01

### Исправлено

- **Kernel panic на реальном железе (heap.rs:101, overflow).** Причина —
  NVMe-драйвер выдавал один PRP1 на весь запрос. Передачи больше страницы
  (например, чтение FAT32-кластера 32 КиБ при сканировании boot-лога на
  системном NVMe) заставляют контроллер писать за пределы DMA-буфера и
  затирать заголовки кучи. Теперь `read_dev`/`write_dev` режут I/O на куски
  не больше 4096 байт (PRP1-only) и добавляют проверки границ.
- **Куча стала самодиагностируемой:** у каждого свободного узла есть magic,
  проверяются размер и границы. При повреждении ядро печатает
  `[HEAP] CORRUPT node ...` и отрезает испорченный хвост вместо panic.
- `deallocate` больше не создаёт узел меньше заголовка FreeNode: размер
  блока считается как `max(request, MIN_BLOCK)` и одинаково в alloc/dealloc.
- MADT x2APIC: флаги читались по смещению 12 (ACPI UID) вместо 8; число
  включённых CPU могло быть случайным.
- `diag heap-check` — проверка целостности free-list; `vktst run mem` и
  `make nvme-test` теперь её выполняют.
- `make nvme-test` создаёт FAT32 с кластерами 32 КиБ и читает файл 200 КиБ,
  то есть реально проверяет многократные NVMe-передачи.

## 0.2.0-rc.5 «Foundation RC5» — 2026-10-31

### Файловые системы

- ext2-драйвер читает **ext4**: extent-дерево, 64-битные group descriptors,
  `flex_bg`, `dir_index` (линейный обход htree), `metadata_csum`, `large_dir`.
  Неподдерживаемые `incompat`-фичи (`inline_data`, `encrypt`, `bigalloc`,
  `meta_bg`, `compression`) теперь дают `NotSupported`, а не «успешное»
  монтирование с пустым каталогом.
- `detect_fs_type` различает `ext2`/`ext3`/`ext4`; `mount auto` и
  `mount ... ext4` сообщают реальный тип.
- MBR: разбирается цепочка extended-разделов (EBR), поэтому логические
  NTFS/FAT-разделы больше не остаются «unknown».
- Protective MBR больше не обрывает разбор таблицы: hybrid-образы
  (`iso-debug`) с appended-разделами определяются корректно.
- GPT и смещения разделов приведены к 512-байтным единицам — исправлены
  4Kn-диски.
- Относительные пути (`cat hello.txt`, `ls sub`, `tree`) разрешаются
  относительно текущего каталога.

### Загрузка и образы

- Добавлен ATAPI/CD-драйвер поверх AHCI: `PACKET`, `READ CAPACITY(10)`,
  `READ(10)`, блоки 2048 байт, регистрация как `sr0`.
- Debug-образы имеют три раздела: `VIBRA-ESP` (Limine), `VIBRA` (ядро,
  `fslabel(VIBRA):/kernel.elf`) и `VIBRALOG` (FAT32, `/VIBRA.LOG`).
- Boot-лог честно сообщает, что CD/DVD только для чтения, и предлагает
  записать ISO на USB.
- Имена артефактов содержат дату и версию:
  `vibra_2026-10-31_0.2.0-rc.5.iso`,
  `vibra-debug_2026-10-31_0.2.0-rc.5.iso`,
  `vibra-uefi_2026-10-31_0.2.0-rc.5.img`,
  `vibra-usb_2026-10-31_0.2.0-rc.5.img`,
  `vibra-usb-debug_2026-10-31_0.2.0-rc.5.img`.
- `version`/`uname -a` показывают дату сборки, git commit и профиль.

### Система и shell

- CPU topology: CPUID `0x1F`/`0x0B`, AMD `0x80000008`/`0x8000001E`, fallback
  на MADT. `12C / 24T` определяется корректно; CPUID.1.EBX[23:16] больше не
  выдаётся за число ядер.
- Загрузка CPU считается по времени в idle (`hlt`), а не по числу
  context switch. На простое — 0–2%, а не 24%.
- `top` переписан в стиле htop: CPU, память, heap, дисковый I/O, сеть,
  процессы с CPU% и временем; `top -b` для CI.
- `free`/`top`/`neofetch` используют один `meminfo()` и формат GiB/MiB.
- `tree` без аргумента идёт от текущего каталога.
- `Ctrl+C` и `Ctrl+Z` отменяют длинные команды (`tree`, `cp`, `rm`, `top`,
  `vktst`).
- `vktst` (Vibra Kernel Tests): интерактивное меню, `vktst list`,
  `vktst run <id|all>`, `vktst report` с сохранением в
  `/var/log/vktst-<uptime>.log`.
- `disks` / `disks -v`: блочные устройства, разделы, смещения и тип ФС —
  быстрый ответ на «диск виден, но ФС unknown».
- Консоль переводит Unicode-рамки в CP437, поэтому `tree`, `ps` и таблицы
  снова рисуют `├──`, `│` и `─`, а не пустые ячейки.
- Список команд сокращён с 54 до 42: удалены дубли и dev-заглушки `test`,
  `systest`, `tasks`, `kstat`, `apic`, `display`, `heap`, `hidtest`,
  `usertest`, `test-disk`, а также фиктивные `chmod` и `passwd`
  («режим установлен»/«пароль принят», хотя ничего не менялось).
  `apic`/`display` перенесены в `diag apic`/`diag display`; `df` теперь
  показывает реальные монтирования вместо выдуманных размеров.
- Удалён мёртвый legacy-код RAM-диска (`RamDisk`, `FatDisk`, `add_disk`) и
  неиспользуемые хелперы `fs_count`/`list_entries`; бюджет предупреждений
  снижен с 107/110 до 85/88, фактически 76.

### CI

- Новые QEMU-тесты: `vktst-test`, `debug-image-test`, `iso-debug-test`.
- `.github/workflows/release.yml`: сборка release/debug ISO, UEFI/USB образов,
  `SHA256SUMS` и публикация GitHub Release по тегу `v*`.

### Известные ограничения

- ext2/3/4 — **только чтение**. Запись возвращает `ReadOnly`.
- NTFS — только чтение.
- SMP не включён: `top` показывает агрегированную загрузку и помечает
  остальные логические CPU как неактивные.
- Запись boot-лога на CD/DVD физически невозможна; используйте USB-образ или
  ISO, записанный на флешку.

## 0.2.0-rc.4 «Foundation RC4»

- Стабильный UEFI/Limine boot, NVMe, 4Kn/GPT, mount aliases, shell redraw,
  boot-лог на USB, QEMU CI.

## 0.2.0-rc.3 «Foundation RC3»

- 4Kn/GPT, mount aliases и shell redraw.

## 0.2.0-rc.2 «Foundation RC2»

- NVMe, reboot, shell и logging fixes.

## 0.1.0 «Bootstrap»

- Первый публичный каркас ядра: UEFI/Limine, serial, framebuffer, PMM, heap,
  GDT/IDT, PS/2 и диагностическая shell.
