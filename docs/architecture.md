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
| `fs` | RamFS, VFS, прототипы FAT32/Ext2 | нет общего подключённого block layer |
| `net` | Ethernet/ARP/IPv4/ICMP | QEMU-ориентированный, без DHCP/TCP |

## Ограничения архитектуры

- ARM64-код является заглушкой и не входит в сборку/поддержку.
- DMA-контракт и ownership страниц ещё не унифицированы.
- Нет готового драйверного lifecycle, partition layer и автомонтирования.
- GUI и `vibra/` не являются ядром release scope.

Следующий архитектурный шаг — общий read-only `BlockDevice`, затем USB HID и
реальная валидация на x86_64. См. [roadmap.md](roadmap.md).
