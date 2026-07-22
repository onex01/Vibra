# Roadmap — План развития Vibra OS

> **Текущая версия:** 0.7.0 "Photon" (ядро) / 0.7.0 "Rigel" (ОС).

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

### Фаза 1.2 — Hardware drivers (для реального железа):
- [x] PCI enumeration — config space 0xCF8/0xCFC, bus scan, BAR/IRQ, lspci command
- [x] AHCI/SATA driver — HBA init, port scan, ATA READ DMA EXT, FIS+PRD
- [x] AHCI DiskIo — read/write через AHCI, DiskIo trait implementation
- [x] CPUID — brand string, core count, TSC frequency, asm cpuid wrapper
- [ ] USB (XHCI/EHCI) — клавиатура/мышь для modern hardware
- [ ] NVMe driver — быстрый доступ к SSD
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

### Фаза 3 — APIC вместо PIC (v0.7.5) — КРИТИЧЕСКИЙ БЛОКЕР:
- [x] LAPIC: детект (CPUID), MSR enable, SVR=0x1FF, TPR=0, LINT disabled
- [x] IO APIC: детект, все 24 IRQ замаскированы
- [x] APIC infrastructure: detect, EOI, IO APIC redirect API, LAPIC timer API
- [x] HHDM-based MMIO: LAPIC/IO APIC через HHDM offset
- [x] Assembly MMIO: LAPIC read/write через inline asm (fix serial blocking)
- [x] LAPIC timer калибровка через PIT channel 2
- [x] LAPIC timer start (periodic, vector 48)
- [x] IO APIC mask/unmask/redirect API
- [x] IDT: timer=v32 (PIC), kbd=v33 (PIC), softirq=v0x81, spurious v0xFF
- [x] EOI smart: tick_and_switch и isr_keyboard проверяют APIC_ACTIVE
- [x] Команда `apic` — show status
- [ ] Полная миграция: IRQ0 → LAPIC timer(v48), IRQ1 → IO APIC GSI1(v33)
- [ ] pic::mask_all() после полной миграции

### Фаза 4 — Ring 3 + syscall (v0.8.0):
- [x] syscall/sysret (GDT: USER_DS=0x1B, USER_CS=0x23, STAR[63:48]=0x13)
- [x] `src/syscall/mod.rs`: MSR (EFER.SCE, STAR, LSTAR, SFMASK), naked syscall_entry
- [x] Сисколлы: write / exit / yield с валидацией user-указателей
- [x] `src/task/user.rs`: user-задача (код через iretq, стек NX+RW)
- [x] TSS.rsp0 обновляется при переключении задач
- [x] `isr_page_fault`: user-фолт убивает задачу, а не вешает ОС
- [x] Команда `usertest` — запуск user-space процесса

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
- [ ] USB (XHCI/EHCI) для реального железа
- [ ] NVMe для современных SSD
- [ ] AHCI write (запись на SATA диски)
- [ ] AHCI/VirtIO → VFS integration (boot с диска)
- [ ] Драйверы GPU (.basic framebuffer UEFI → VESA → DRM)
- [ ] Сетевой стек TCP/IP через e1000/VirtIO-net
