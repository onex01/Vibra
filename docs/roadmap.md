# Roadmap — План развития Vibra OS

> **Текущая версия:** 0.5 «Nucleus» (см. `src/version.rs`).
> Заметка по техническим долгам: в `src/main.rs` баннер пока жёстко кодирует
> «0.4 Photon» — привести к единому источнику истины (`version.rs`) отдельно.

## Версия 0.3 "Cartridge"

### Цели:
- [x] PS/2 клавиатура (базовый ввод)
- [x] RamFS (файловая система в памяти)
- [x] Простейший shell с командами:
  - `ls` — список файлов
  - `cat` — чтение файла
  - `echo` — запись в файл
  - `rm` — удаление файла
  - `help` — справка

## Версия 0.4 "Photon"

### Цели:
- [x] Поддержка папок в RamFS
- [x] Команды: `mkdir`, `cp`, `mv`, `cd`, `edit`, `touch`, `pwd`
- [x] Свои GDT + TSS + IST-стеки (Double Fault, NMI)
- [x] IDT: исключения CPU + IRQ0 timer + IRQ1 keyboard, PIC, PIT
- [x] Подсистема `kernel`: device/driver/module/registry/event
- [ ] FAT32 драйвер (чтение/запись)
- [ ] Виртуальная файловая система (VFS)

## Версия 0.5 "Nucleus" (текущая)

### Цели (этап «Фундамент + память»):
- [x] PMM с локами + next-fit + `alloc_contiguous` + `alloc_frame_zeroed` + `stats`
- [x] Heap: собственный free-list аллокатор с коалесценцией (бэкенд PMM + HHDM)
- [x] Heap-стресс (10k alloc/drop) и shell-команды `heap` / `diag pmtest`
- [~] Собственные page tables: готовы CR3/PML4 walker и проверка неактивной PML4 с private 4КБ mapping; далее HHDM + ядро 4КБ-страницами + framebuffer — Шаг 4
- [ ] Проверка NX/WX-страниц (`wxtest`/`nxtest`)
- [ ] Вытесняющий планировщик (kernel-threads) — следующий этап
- [ ] Базовый драйвер AHCI (для QEMU) или VirtIO Block (гораздо проще для начала).

### Фаза 0 — Починка сборки (ядро 0.5.6) — СДЕЛАНО
Ядро не собиралось: 36 ошибок компиляции в WIP-коде VFS (`src/fs/`, `src/commands/`).
Из-за этого «не работали прерывания» — сам код прерываний (`src/interrupts/`) исправен.

Что исправлено (работа над VFS сохранена, не откачена):
- [x] Импорты `alloc` (`Box`/`Vec`/`vec`) в `fs/disk.rs`, `fs/mod.rs`, `commands/mount.rs`, `commands/test_disk.rs`
- [x] Глобальные менеджеры без `Option`: `Lazy<VfsManager>` и `Lazy<Mutex<DiskManager>>` (`fs/mod.rs`)
- [x] `MountTable::find_fs` возвращает `(usize, String)` вместо ссылок из локальных переменных; добавлены `get`/`get_mut`/`is_mounted`; удалён битый `VfsManager::find_fs`
- [x] Удалён дублирующий трейт `DiskIo` в `fat32.rs` — теперь `Fat32Fs` хранит `Box<dyn DiskIo>` из `disk.rs`
- [x] Удалён дубликат `DiskManager` в `disk.rs` (канон — `disk_manager.rs`) и мёртвый `legacy_fs.rs`
- [x] Исправлено затенение `disk` в `test_disk.rs` (диск добавлялся сам в себя)
- [x] Вычищены warnings в затронутых файлах; остались только «never used» для VFS-API (подключится в следующих фазах)

Проверка: `cargo +nightly build` — 0 ошибок; `make build` — ядро установлено в `build/hdd.img`.
Регресс-тест в QEMU (`make run`): `mount`, `test-disk ram 4`, `test-disk list`, `touch a; ls; cat a`, `diag`, `heap`.

## План Kernel-First (следующие фазы)

> Полный план с деталями: `.mimocode/plans/1784391303907-hidden-island.md`.
> Стратегия: сначала мощное ядро, GUI/приложения — после (v1.0+).

### Фаза 1 — Собственные page tables (ядро → 0.6.0) — СЛЕДУЮЩАЯ
- [ ] Новый файл `src/memory/vmm.rs`: построить свой PML4 с нуля (не копию Limine)
- [ ] Символы границ секций в `linker.ld` (`__text_start/end`, `__rodata_*`, `__data_*`, `__bss_end`)
- [ ] `ExecutableAddressRequest` из limine для phys/virt базы ядра
- [ ] W^X: `.text` — исполняемый/RO, `.rodata` — NX/RO, `.data/.bss` — NX/RW
- [ ] HHDM 2МиБ-страницами по memmap; ОБЯЗАТЕЛЬНО замапить BOOTLOADER_RECLAIMABLE (там стек Limine — иначе triple fault) и framebuffer
- [ ] Задел под APIC: MMIO `hhdm+0xFEE00000` / `hhdm+0xFEC00000` (4КиБ, PCD)
- [ ] Активация: EFER.NXE (бит 11) ДО `mov cr3`; всё до `interrupts::enable()`
- [ ] W^X-подтест в `diag`: запись в `.rodata` → аккуратный PAGE FAULT
- [ ] Подсистема ввода: унифицированные структуры событий (Key, MouseMove, MouseClick).
- [ ] Виртуальные устройства 

### Фаза 2 — Вытесняющий планировщик (ядро → 0.7.0)
- [ ] `src/task/mod.rs` (TCB, Scheduler, spawn/yield/sleep/exit) + `src/task/switch.rs`
- [ ] Naked-стаб для вектора 32 вместо `extern "x86-interrupt"` (push 15 GP-регистров → смена rsp → iretq)
- [ ] Вектор 0x81 для yield/exit; задача 0 = kshell, задача 1 = idle
- [ ] Round-robin, квант 1 тик (10 мс); `try_lock` в тике против дедлока; без аллокаций в ISR
- [ ] Команда `tasks` показывает реальный список задач

### Фаза 3 — APIC вместо PIC (ядро → 0.7.5)
- [ ] `src/interrupts/apic.rs`: LAPIC (SVR, EOI) + IO APIC (GSI1 → вектор 33 для клавиатуры)
- [ ] LAPIC-таймер, калибровка по PIT-каналу 2 (polling порта 0x61), те же 100 Гц
- [ ] 8259: только ремап + полная маскировка (`mask_all()`); без PIC-fallback
- [ ] Чтение CMOS/RTC для инициализации времени.
- [ ] Системный вызов sys_clock_gettime и sys_nanosleep.

### Фаза 4 — Ring 3 + syscall (ядро → 0.8.0)
- [ ] syscall/sysret (GDT уже спроектирована: USER_DS=0x1B, USER_CS=0x23, STAR[63:48]=0x13)
- [ ] `src/syscall/mod.rs`: MSR (EFER.SCE, STAR, LSTAR, SFMASK), naked syscall_entry
- [ ] Сисколлы: write / exit / yield с валидацией user-указателей
- [ ] `src/task/user.rs`: user-задача (код 0x400000 RO+X, стек NX+RW), вход через iretq
- [ ] TSS.rsp0 обновляется при переключении задач (иначе triple fault на первом IRQ в ring 3)
- [ ] `isr_page_fault`: user-фолт убивает задачу, а не вешает ОС; команда `usertest`
- [ ] Реализация sys_mmap (MAP_ANONYMOUS) и sys_munmap для динамического выделения памяти в user-space.
- [ ] Парсер ELF64 в ядре.
- [ ] Создание user-space процесса из ELF-файла с диска (через VFS).
- [ ] Простой механизм IPC (например, message passing или shared memory через sys_mmap с флагом MAP_SHARED).
- [ ] Системный сервис или syscall для буфера обмена.

## Версия 1.0 "Nova"

### Цели:
- [ ] Графический интерфейс (оконный менеджер)
- [ ] Базовые приложения:
  - Файловый менеджер
  - Текстовый редактор
  - Калькулятор
- [ ] Поддержка мыши

## Версия 2.0 "Andromeda"

### Цели:
- [ ] Сетевой стек (TCP/IP)
- [ ] Браузер (NetSurf или Ladybird)
- [ ] Аудиодрайвер
- [ ] Видеоплеер

## Версия 2.1 "Capricorn"

### Цели:
- [ ] Порт на ARM64 (Orange Pi Zero 3)
- [ ] Поддержка эмуляторов:
  - NES (FCEUX)
  - SNES (Snes9x)
  - GBA (mgba)

## Долгосрочные планы

- [ ] Собственный компилятор (TCC для C или интерпретатор Lua)
- [ ] IDE с подсветкой синтаксиса
- [ ] Поддержка многозадачности
- [ ] Защита памяти (user mode)
- [ ] Драйверы USB, SATA, GPU
