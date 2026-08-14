# Архитектура Vibra OS

## Ядро

**Тип:** Модульное монолитное ядро  
**Язык:** Rust (no_std)  
**Загрузчик:** Limine (UEFI)

### Модули ядра:

1. **serial** — драйвер последовательного порта COM1 (0x3F8)
2. **gdt** — собственные GDT + TSS + IST-стеки (Double Fault, NMI)
3. **interrupts** — IDT (исключения CPU + IRQ0 timer + IRQ1 keyboard), PIC, PIT
4. **memory::pmm** — физический менеджер памяти (bitmap allocator, next-fit, локи)
5. **memory::heap** — аллокатор кучи (free-list с коалесценцией поверх PMM+HHDM)
6. **memory::paging / memory::vmm** — обход page tables, собственная PML4 и переключение CR3
7. **keyboard** — драйвер PS/2 клавиатуры (порт 0x60, IRQ1)
8. **fs** — VFS, RamFS, FAT32, Ext2, procfs, sysfs, devtmpfs
9. **shell** — командная оболочка с line editor и tab-completion

## Загрузка

1. UEFI BIOS (OVMF) загружает Limine
2. Limine читает `limine.conf` и загружает `kernel.elf`
3. Limine передаёт ядру:
   - Framebuffer (адрес, размер, pitch)
   - Memory map (карта памяти)
   - HHDM offset (Higher Half Direct Map)
4. Ядро инициализирует подсистемы в строгом порядке зависимостей:
   serial → PMM → heap (alloc_contiguous 4 МБ) → heap-стресс → ACPI →
   keyboard/input/devices → PCI/AHCI/NVMe/USB/net drivers → timer → fs →
   kernel/users/task → VMM → GDT → IDT/PIC/APIC → syscall → `sti`
5. Входит в главный цикл shell

## Управление памятью

Подробное описание — в [`memory.md`](memory.md).

- **Физическая память (PMM):** битмап-аллокатор (1 бит на 4 КБ-фрейм), next-fit
  поиск, локи через `without_interrupts`. Намеренно не раздаёт
  `BOOTLOADER_RECLAIMABLE` (там стек BSP).
- **Куча (heap):** собственный free-list аллокатор (~250 строк) с address-ordered
  списком и коалесценцией соседей. Бэкенд — регион из PMM (4 МБ), отображённый
  через HHDM. Поддерживает `free` (в отличие от прежнего BumpAllocator).
- **Виртуальная память:** `memory::paging` умеет обходить таблицы страниц, а
  `memory::vmm` строит собственную PML4 на базе карты Limine, включает NX/WP и
  переключает CR3. Диагностика W^X/NX доступна через shell-команды.

## Ввод-вывод

- **Serial:** Port-mapped I/O (0x3F8)
- **Serial debug shell:** development feature `serial-debug` читает COM1
  polling-ом и зеркалит shell в порт; `--no-default-features` исключает этот
  канал ввода из образа.
- **Framebuffer:** Memory-mapped I/O (через HHDM)
- **Клавиатура:** PS/2 (порт 0x60, прерывание IRQ1)

## Файловая система

### RamFS (реализована)
- Хранение в памяти на статических массивах (`[u8; 4096]`, `[Option<FsEntry>; 64]`)
- Поддержка файлов и каталогов
- Heap не использует — полностью статическая
- Команды: `ls`, `cat`, `touch`, `mkdir`, `cd`, `cp`, `mv`, `rm`, `edit`

### Дисковые ФС
- FAT32: чтение/запись, BPB parse, cluster-chain, каталоги
- Ext2: чтение superblock/inode/dirent, direct blocks
- VFS mount table объединяет RamFS, procfs, sysfs, devtmpfs и дисковые ФС

### Следующий шаг
- Автоматически находить boot-диск, разбирать MBR и монтировать FAT32-раздел в `/mnt/boot`
