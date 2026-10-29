# Архитектура Vibra Kernel

## Область действия

Vibra Kernel 0.2.0-rc.4 — экспериментальное модульное монолитное ядро для
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
CMOS), `kstat`, `uptime`.

## Подсистемы

| Блок | Роль | Статус |
|---|---|---|
| `memory` | PMM, heap, page tables, VMM | экспериментальный |
| `interrupts`, `gdt` | IDT, PIC, APIC-задел, TSS | PIC baseline; LAPIC timer только как fallback для тиков |
| `timer` | TSC/HPET/PIT часы, CMOS RTC, wall clock | TSC + HPET, RTC синхронизирован; IRQ-тики измеряются |
| `drivers/pci`, `acpi` | обнаружение x86-оборудования | PCI + ACPI; BAR выше 4 ГБ переносятся в низкое MMIO-окно |
| `drivers/usb` | xHCI lifecycle + HID boot keyboard | enumeration и HID-отчёты работают в QEMU |
| `fs` | RamFS, VFS, block layer, FAT32/Ext2/NTFS (ro) | block layer + разделы + mount работают в QEMU |
| `net` | Ethernet/ARP/IPv4/ICMP | QEMU-ориентированный, без DHCP/TCP |

## Ограничения архитектуры

- ARM64-код является заглушкой и не входит в сборку/поддержку.
- DMA-контракт и ownership страниц ещё не унифицированы.
- Нет единого driver lifecycle: AHCI/NVMe/xHCI инициализируются разными путями,
  а запись на диск пока не поддерживается ни одной ФС (только read-only).
- GUI и `vibra/` не являются ядром release scope.
- NX/XD опционален: `EFER.NXE` включается только при `CPUID.80000001H:EDX[20]`.
  Если в BIOS выключен «Execute Disable Bit», ядро не пишет MSR (иначе `#GP(0)`)
  и не ставит NX-биты в PTE — W^X деградирует, но boot продолжается.

Следующие архитектурные шаги — PIC → LAPIC/IOAPIC и политика MSI/MSI-X, затем
DMA-контракт для блочных драйверов и DHCP/ICMP. USB HID keyboard и xHCI
lifecycle реализованы (см. [usb_progress.md](usb_progress.md)), но ещё не
подтверждены на физической машине. См. [roadmap.md](roadmap.md).
