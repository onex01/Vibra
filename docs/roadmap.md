# Roadmap — План развития Vibra OS

> **Текущая версия:** 0.6 «Nucleus» (см. `src/version.rs`).

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

## Версия 0.5 "Nucleus"

### Цели:
- [x] PMM с локами + next-fit + `alloc_contiguous` + `alloc_frame_zeroed` + `stats`
- [x] Heap: собственный free-list аллокатор с коалесценцией (бэкенд PMM + HHDM)
- [x] Heap-стресс (10k alloc/drop) и shell-команды `heap` / `diag pmtest`
- [x] Собственные page tables: CR3/PML4 walker и проверка неактивной PML4 с private 4КБ mapping
- [x] Проверка NX/WX-страниц (`wxtest`/`nxtest`)
- [x] Вытесняющий планировщик (заглушка) — следующий этап
- [x] Базовый драйвер VirtIO Block (probe + config read)

## Версия 0.6 "Nucleus" (текущая)

### Цели (Фаза 1 — VMM):
- [x] VMM: копирование PML4 Limine + переключение CR3
- [x] EFER.NXE + CR0.WP включены
- [x] Символы секций в linker.ld
- [x] `ExecutableAddressRequest` из Limine
- [x] Подсистема ввода: `src/input.rs` (Key, MouseMove, MouseClick)
- [x] Виртуальные устройства: `src/devices/` (VirtIO Block, Net, PC Speaker)
- [x] Планировщик задач (заглушка): `src/task/mod.rs` (round-robin)
- [x] W^X-подтест в `diag wxtest`

### Фаза 1.1 — Исправления и улучшения:
- [ ] Исправить PS/2 keyboard input через framebuffer
- [ ] W^X remap поверх скопированного PML4 (demote 2MB → 4KB)
- [ ] VirtIO Block: полная реализация VRing I/O
- [ ] FAT32 драйвер (чтение/запись)
- [ ] VFS: улучшения и интеграция с VirtIO

## План Kernel-First (следующие фазы)

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
