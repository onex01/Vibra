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
6. **memory::paging** — read-only обход текущих page tables Limine (CR3, virt → phys)
7. **keyboard** — драйвер PS/2 клавиатуры (порт 0x60, IRQ1)
8. **fs** — файловая система (RamFS на статических массивах; FAT32 — планируется)
9. **shell** — командная оболочка с line editor и tab-completion

## Загрузка

1. UEFI BIOS (OVMF) загружает Limine
2. Limine читает `limine.conf` и загружает `kernel.elf`
3. Limine передаёт ядру:
   - Framebuffer (адрес, размер, pitch)
   - Memory map (карта памяти)
   - HHDM offset (Higher Half Direct Map)
4. Ядро инициализирует подсистемы в строгом порядке зависимостей:
   serial → PMM → heap (alloc_contiguous 4 МБ) → heap-стресс → keyboard →
   fs → kernel (первые постоянные heap-аллокации) → device/driver →
   GDT → IDT/PIC/PIT → `sti`
5. Входит в главный цикл shell

## Управление памятью

Подробное описание — в [`memory.md`](memory.md).

- **Физическая память (PMM):** битмап-аллокатор (1 бит на 4 КБ-фрейм), next-fit
  поиск, локи через `without_interrupts`. Намеренно не раздаёт
  `BOOTLOADER_RECLAIMABLE` (там стек BSP).
- **Куча (heap):** собственный free-list аллокатор (~250 строк) с address-ordered
  списком и коалесценцией соседей. Бэкенд — регион из PMM (4 МБ), отображённый
  через HHDM. Поддерживает `free` (в отличие от прежнего BumpAllocator).
- **Виртуальная память:** пока используются таблицы страниц Limine (HHDM маппит
  всю физику). `memory::paging` уже проверяет CR3 и реальные отображения в
  read-only режиме; переключение на собственные page tables — следующий подшаг.

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

### FAT32 (планируется)
- Чтение/запись на диск
- Поддержка папок
- Совместимость с Windows/Linux
