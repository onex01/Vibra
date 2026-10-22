# Архитектура Vibra Kernel

## Область действия

Vibra Kernel 0.2.0-rc.1 — экспериментальное модульное монолитное ядро для
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

## Подсистемы

| Блок | Роль | Статус |
|---|---|---|
| `memory` | PMM, heap, page tables, VMM | экспериментальный |
| `interrupts`, `gdt` | IDT, PIC, APIC-задел, TSS | PIC baseline; миграция не завершена |
| `drivers/pci`, `acpi` | обнаружение x86-оборудования | частично реализовано |
| `drivers/usb` | xHCI probe | не выполняет USB enumeration/HID |
| `fs` | RamFS, VFS, block layer, FAT32/Ext2/NTFS (ro) | block layer + разделы + mount работают в QEMU |
| `net` | Ethernet/ARP/IPv4/ICMP | QEMU-ориентированный, без DHCP/TCP |

## Ограничения архитектуры

- ARM64-код является заглушкой и не входит в сборку/поддержку.
- DMA-контракт и ownership страниц ещё не унифицированы.
- Нет единого driver lifecycle: AHCI/NVMe/xHCI инициализируются разными путями,
  а запись на диск пока не поддерживается ни одной ФС (только read-only).
- GUI и `vibra/` не являются ядром release scope.

Следующие архитектурные шаги — PIC → LAPIC/IOAPIC и политика MSI/MSI-X, затем
полный xHCI lifecycle и USB HID keyboard. См. [roadmap.md](roadmap.md).
