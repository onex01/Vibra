# Roadmap — Vibra Kernel

> Текущая версия: **0.1.0 «Bootstrap»**.  
> Цель: стабильное диагностическое ядро на реальном x86_64-железе, не ОС.

## Принципы

- Новая версия возможна только после воспроизводимого теста, а не после
  добавления прототипа драйвера.
- До `0.5.0` GUI, приложения, userland и ARM64 не расширяют scope.
- Операции записи на физический диск выключены по умолчанию и требуют
  отдельного тестового носителя.
- QEMU — регрессия; реальная машина — критерий готовности.

## 0.1.0 «Bootstrap» — текущая база

- [x] UEFI/Limine boot path в QEMU.
- [x] Serial, framebuffer, PMM, heap, GDT/IDT, PS/2 и диагностическая shell.
- [x] Компиляция `x86_64-unknown-none`.
- [ ] Единый smoke-тест boot-лога и инвентаризация hardware claims.

## 0.2.0 «Foundation» — воспроизводимый x86_64 boot

- [ ] Чёткий kernel-only build layout и единая версия.
- [ ] Panic/page-fault отчёт в serial с регистрами и адресом ошибки.
- [ ] Контроль warnings и QEMU smoke-test в CI.
- [ ] Проверяемые PMM/heap stress-тесты; DMA-аллокатор с физическими адресами.
- [ ] Матрица: QEMU UEFI и минимум одна реальная x86_64-машина.

**Выход:** 100 последовательных QEMU boot и 10 boot на тестовом ПК без
повреждения носителя.

## 0.3.0 «Hardware Core» — прерывания и блочный слой

- [ ] Завершить PIC → LAPIC/IOAPIC для BSP; определить политику MSI/MSI-X.
- [ ] Общий `BlockDevice`: bounds, timeouts, read-only, ошибки и DMA-буферы.
- [ ] GPT/MBR parser и partition devices.
- [ ] Один контроллер хранения: AHCI **или** NVMe, только чтение.
- [ ] Привязать найденные диски к VFS.

## 0.4.0 «Peripherals» — ввод и файловая система

- [ ] Полный xHCI lifecycle: reset, DCBAA, command/event rings, enumeration.
- [ ] USB HID boot keyboard на физической клавиатуре.
- [ ] FAT32 read-only, затем Ext2 read-only поверх общего block layer.
- [ ] Автомонтирование явно выбранного read-only boot-раздела.

## 0.5.0 «Hardware Ready» — стабильное ядро

- [ ] NIC для одного поддержанного физического адаптера.
- [ ] RX/TX, ARP, DHCP, IPv4 и ICMP; `ping` внешнего узла с тайм-аутом.
- [ ] Regression matrix минимум для двух x86_64-конфигураций.
- [ ] Документированная recovery-процедура и boot-логи.

**Выход:** с USB-образа загружается диагностика, работает USB keyboard,
чтение с диска и DHCP/ping на двух реальных машинах.

## После 0.5.0

- `0.6`: controlled write path, USB Mass Storage, UDP и собственный remote shell.
- `0.7`: SMP и пер-CPU ресурсы после аудита локов.
- `0.8+`: ARM64 platform layer. Первый этап Orange Pi Zero 3 (H618): UART,
  U-Boot, Device Tree, MMU, GIC, generic timer, SD/MMC и Ethernet.
  Mali/HDMI/Wi-Fi не входят в первый ARM milestone.
- GUI и desktop возможны только после отдельного решения о userland ABI.
