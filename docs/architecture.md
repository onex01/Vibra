# Архитектура Vibra Kernel

## Область действия

Vibra Kernel 0.2.0-rc.25.5 — экспериментальное модульное монолитное ядро для
`x86_64-unknown-none`. Оно загружается Limine в UEFI и использует QEMU как
основной тестовый стенд. Реальное оборудование пока не считается
поддержанным; каждый драйвер должен получить отдельную hardware-валидацию.

## Текущий boot path

1. Limine передаёт framebuffer, memory map, HHDM, executable address и ACPI RSDP.
2. Ядро запускает serial, PMM и heap; затем ACPI, PCI-пробы и диагностические
   прототипы драйверов.
3. Создаются VMM, GDT, IDT и системные обработчики; включаются прерывания.
4. Запускается framebuffer/serial shell.

Этот порядок не означает, что все вызываемые драйверы готовы к железу: xHCI,
диски и NIC должны быть отделены от boot-critical пути и получить тайм-ауты,
DMA-контракты и обработку ошибок.

### Ввод на реальном ноутбуке

Встроенная клавиатура ноутбука почти всегда подключена к i8042 (EC), а не к USB.
`keyboard.rs` поэтому не полагается на IRQ1:

- `init()` выполняет self-test 8042, включает порт (0xAE) и scanning (0xF4),
  проверяет, что контроллер принял translation в Scan Code Set 1;
- при отказе translation драйвер декодирует Scan Code Set 2;
- `poll_key()` дополнительно опрашивает порт 0x64 (polling fallback), поэтому
  ввод работает, даже если IRQ1 не доставляется (ACPI/EC, IOAPIC вместо PIC);
- ISR не вызывает `println!`: serial/framebuffer используют spin-локи, и печать
  из прерывания вешает машину, если лок уже взят основным кодом.

`diag keyboard` печатает состояние контроллера, scan set, счётчик IRQ1, очереди
PS/2 и USB HID и число HID-отчётов.

### Таймер и время

Время в ядре не зависит от доставки прерываний. Источник выбирается при
`timer::init()` в таком порядке:

1. **TSC**, если `CPUID.80000007H:EDX[8]` подтверждает invariant TSC.
   Частота калибруется по HPET (окно 20 мс) или, если HPET нет, по PIT
   channel 2 (10 мс). Это основной источник на реальном железе: чтение
   `rdtsc` не требует MMIO и не «плывёт» от частоты IRQ.
2. **HPET** — 64-битный основной счётчик. Частота берётся из поля PERIOD
   регистра ID (`1e15 / period_fs`), а не по PCI vendor ID. Бит `ENABLE_CNF`
   (0) включается явно, legacy replacement (1) выключается, чтобы не
   подменять PIT IRQ0.
3. **PIT IRQ0** (100 Гц) — последний вариант, только если нет ни TSC, ни HPET.

`interrupts::idt::TICKS` — это тики планировщика, а не время. `uptime`,
`date`, тайм-ауты ARP/ICMP и мигание курсора считаются по `timer::uptime_ms()`
/ `uptime_ns()`.

Время BIOS читается из CMOS (`0x70/0x71`) в `timer/rtc.rs`: ожидание UIP,
двойное чтение до совпадения, BCD/12-часовой режим, век из поля CENTURY
таблицы FADT. При старте фиксируется пара «Unix-время RTC + монотонные
наносекунды», поэтому `date` показывает реальные дату и время, а не uptime.

После `interrupts::enable()` вызывается `timer::post_init()`: он измеряет
фактическую частоту IRQ-тиков. Если IRQ0 не доставляется вообще (PIC не
подключён, таймер уходит в IOAPIC), поднимается LAPIC timer на векторе 48
(калибровка по монотонным часам, EOI в `isr_timer_apic`), а PIC IRQ0
маскируется. Если и это невозможно, `timer::idle_wait()` не делает `hlt`
(он не вернулся бы без прерываний), а крутится в `spin_loop` — shell
остаётся живым на polling-вводе.

Диагностика: `diag timer` (источник и частота, HPET/TSC, измеренная частота
IRQ, RTC, расхождение IRQ-времени с аппаратным), `date hw` (сырые регистры
CMOS), `diag apic`, `uptime`.

## Подсистемы

| Блок | Роль | Статус |
|---|---|---|
| `memory` | PMM, heap, page tables, VMM | экспериментальный |
| `interrupts`, `gdt` | IDT, PIC, APIC-задел, TSS | PIC baseline; LAPIC timer только как fallback для тиков |
| `timer` | TSC/HPET/PIT часы, CMOS RTC, wall clock | TSC + HPET, RTC синхронизирован; IRQ-тики измеряются |
| `drivers/pci`, `acpi` | обнаружение x86-оборудования | PCI + ACPI; BAR выше 4 ГБ переносятся в низкое MMIO-окно |
| `drivers/usb` | xHCI + UHCI + EHCI lifecycle, HID boot keyboard, BOT+SCSI mass storage, hotplug | enumeration, ввод и storage работают в QEMU; OHCI нет |
| `fs` | RamFS, VFS, block layer, FAT32 (rw для `VIBRALOG`), ext2/3/4 (ro), NTFS (ro), exFAT/ISO9660 (ro) | block layer + MBR/EBR/GPT + ext4 extents + exFAT/ISO9660 работают в QEMU |
| `drivers/ahci` | SATA + ATAPI CD | ATA read/write и ATAPI `PACKET`/`READ(10)` работают в QEMU |
| `drivers/ide` | PATA/IDE (BIOS legacy) | только чтение, регистрируется в block layer |
| `drivers/nvme` | NVMe | Identify, I/O queues, 512/4096 байт на блок |
| `commands` | shell-команды | `top` (htop-like), `vktst`, `tree`, `free`, `disks` |
| `net` | Ethernet/ARP/IPv4/ICMP | QEMU-ориентированный, без DHCP/TCP |

## Ограничения архитектуры

- ARM64-код является заглушкой и не входит в сборку/поддержку.
- DMA-контракт и ownership страниц ещё не унифицированы, но адрес для
  PRD/PRP уже берётся из таблиц страниц (`paging::dma_address`), а не
  арифметикой `virt - hhdm`.
- Нет единого driver lifecycle: AHCI/NVMe/ATAPI/xHCI инициализируются разными
  путями. Запись поддерживает только FAT32 (явный `rw`, используется для
  `VIBRALOG`); ext2/3/4, NTFS, exFAT и ISO9660 — read-only.
- SMP не включён: работает только BSP, `top` показывает агрегированную
  загрузку и помечает остальные логические CPU как неактивные.
- GUI и `vibra/` не являются ядром release scope.
- NX/XD опционален: `EFER.NXE` включается только при `CPUID.80000001H:EDX[20]`.
  Если в BIOS выключен «Execute Disable Bit», ядро не пишет MSR (иначе `#GP(0)`)
  и не ставит NX-биты в PTE — W^X деградирует, но boot продолжается.

Следующие архитектурные шаги — PIC → LAPIC/IOAPIC и политика MSI/MSI-X, затем
SMP bring-up, DMA-контракт для блочных драйверов и DHCP/ICMP. xHCI/UHCI/EHCI
lifecycle, USB HID keyboard и USB mass storage реализованы (см.
[usb_progress.md](usb_progress.md)), но подтверждены не на всех физических
машинах: EHCI проверен только в QEMU. См. [roadmap.md](roadmap.md).
