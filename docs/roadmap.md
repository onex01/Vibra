# Roadmap — План развития Vibra OS

> **Текущая версия:** 0.6.4 "Wave" (ядро) / 0.6.1 "Vega" (ОС).

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
- [x] FAT32 драйвер (чтение) — `src/fs/fat32.rs` (~300 строк, реальный BPB parse, cluster-chain, subdir traversal)
- [x] Виртуальная файловая система (VFS) — `src/fs/vfs.rs` + mount table + procfs/sysfs/devtmpfs

## Версия 0.5 "Nucleus"

### Цели:
- [x] PMM с локами + next-fit + `alloc_contiguous` + `alloc_frame_zeroed` + `stats`
- [x] Heap: собственный free-list аллокатор с коалесценцией (бэкенд PMM + HHDM)
- [x] Heap-стресс (10k alloc/drop) и shell-команды `heap` / `diag pmtest`
- [x] Собственные page tables: CR3/PML4 walker и проверка неактивной PML4 с private 4КБ mapping
- [x] Проверка NX/WX-страниц (`wxtest`/`nxtest`)
- [x] Базовый драйвер VirtIO Block (probe + config read)

## Версия 0.6 "Nucleus" (текущая)

### Фаза 1 — VMM:
- [x] VMM: копирование PML4 Limine + переключение CR3
- [x] EFER.NXE + CR0.WP включены
- [x] Символы секций в linker.ld
- [x] `ExecutableAddressRequest` из Limine
- [x] Подсистема ввода: `src/input.rs` (Key, MouseMove, MouseClick)
- [x] Виртуальные устройства: `src/devices/` (VirtIO Block, Net, PC Speaker)
- [x] W^X-подтест в `diag wxtest`

### Фаза 1.1 — FS rewrite:
- [x] FAT32 драйвер (чтение/запись) — реальный BPB parse, cluster-chain, write_cluster/alloc_cluster
- [x] Ext2 драйвер (чтение) — superblock/inode/dirent packed structs, direct blocks[0..12]
- [x] VFS: procfs, sysfs, devtmpfs virtual filesystems
- [x] Shell команды используют VFS API

### Фаза 2 — Вытесняющий планировщик (v0.7.0):
- [x] `src/task/mod.rs` — TCB с kernel stack (8KB), Scheduler, spawn/yield/sleep/exit/task_list
- [x] `src/task/ctx_switch.rs` — naked stubs (vector 32 timer, vector 0x81 softirq)
- [x] Полный save/restore: 15 GP + iretq frame (20 u64 = 160 байт)
- [x] Round-robin, квант 4 тика; `try_lock` в тике против дедлока
- [x] Idle task (hlt-loop) на отдельном kernel stack
- [x] Команда `tasks` показывает реальный список задач
- [x] Команда `ps` — PID, состояние, тики, переключения
- [x] Команда `top` — uptime, tick counter, список задач

### Фаза 3 — APIC вместо PIC (v0.7.5) — В ПРОЦЕССЕ:
- [x] LAPIC: детект (CPUID.01h:EDX bit 9), MSR 0x1B enable, SVR=0x1FF, TPR=0
- [x] IO APIC: все 24 IRQ замаскированы (безопасный старт)
- [x] EOI в naked stub через `tick_and_switch()` — поддержка APIC+PIC
- [x] `isr_lapic_timer` (vector 48) + `isr_serial` (vector 36) в IDT
- [x] **Инкрементальный подход**: PIC остаётся для IRQ0/IRQ1, IO APIC только для serial
- [x] **EOI smart**: `tick_and_switch` и `isr_keyboard` проверяют `APIC_ACTIVE` → PIC или LAPIC EOI
- [x] **LAPIC timer калибровка**: через PIT channel 2 (polling), значение сохраняется
- [x] **IO APIC redirect**: GSI4 → вектор 36 (serial) — работает, PIC не конфликтует
- [x] **Команда `apic`**: status/timer/keyboard/full для ручной миграции
- [ ] **Полная миграция IRQ0**: `apic timer` → mask PIC IRQ0 + IO APIC GSI0→v32 + start LAPIC timer
- [ ] **Полная миграция IRQ1**: `apic keyboard` → mask PIC IRQ1 + IO APIC GSI1→v33
- [ ] **Тест**: keyboard работает через IO APIC, timer через LAPIC, serial через IO APIC

### Фаза 4 — Ring 3 + syscall (v0.8.0):
- [ ] syscall/sysret (GDT: USER_DS=0x1B, USER_CS=0x23, STAR[63:48]=0x13)
- [ ] `src/syscall/mod.rs`: MSR (EFER.SCE, STAR, LSTAR, SFMASK), naked syscall_entry
- [ ] Сисколлы: write / exit / yield с валидацией user-указателей
- [ ] `src/task/user.rs`: user-задача (код 0x400000 RO+X, стек NX+RW), вход через iretq
- [ ] TSS.rsp0 обновляется при переключении задач
- [ ] `isr_page_fault`: user-фолт убивает задачу, а не вешает ОС
- [ ] Реализация sys_mmap (MAP_ANONYMOUS) и sys_munmap
- [ ] Парсер ELF64 в ядре
- [ ] Создание user-space процесса из ELF-файла с диска

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
- [ ] Драйверы USB, SATA, GPU
