# Roadmap — Vibra Kernel

> Текущая версия: **0.2.0-rc.2 «Foundation RC2»**.
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
- [x] QEMU smoke-тест boot-лога и inventory hardware claims; serial log дошёл
  до `vibra:/>` без `[FATAL]`.

## 0.2.0 «Foundation» — воспроизводимый x86_64 boot

- [x] Чёткий kernel-only build layout и единая версия `0.2.0-rc.2`.
- [x] Runtime panic/page-fault checks в изолированной QEMU VM: panic screen,
  CR2, error code, access flags, RIP/RSP/RFLAGS и source location подтверждены.
- [x] Безопасный runtime W^X check: `diag wxtest` подтвердил read-only `.rodata`
  mapping без намеренного kernel fault и остановки гостя.
- [ ] Проверить live boot log на Limine framebuffer и сохранение экрана при
  переходе в shell; реализация добавлена, QEMU-проверка ожидается.
- [x] Silent hang через 30–60 секунд: xHCI polling удалён из timer ISR,
  context-switch stubs больше не сравнивают caller-saved `rdi`, serial dequeue
  защищён от IRQ4, PIT fallback приведён к 100 Hz. Ручной QEMU+xHCI прогон жил
  75 секунд, дошёл до shell и записал 12 heartbeat. Heartbeat выведен на QEMU
  debugcon, COM1 оставлен для shell; после 14 heartbeat `diag pmm` прошёл по COM.
  Дополнительно: `tick_and_switch` шлёт EOI **до** попытки взять `SCHEDULER` —
  иначе занятый лок приводил к потере EOI, таймер замолкал навсегда, а shell
  навсегда оставался в `hlt`. Добавлены маркеры `[TICK]`/`[HALT]` на debugcon.
- [x] QEMU+xHCI heartbeat smoke и panic/page-fault checks добавлены в CI;
  первый CI run ожидается.
- [x] CI сам ставит тулчейн: официальный rustup (пакеты из apt/dnf не дают
  рабочего `rustup`), `nightly --component rust-src`, target
  `x86_64-unknown-none`; PATH передаётся через `$GITHUB_PATH`.
- [x] Warning budgets enforced in CI: debug ≤107, no-default ≤110.
- [x] Растеризация консоли: строка глифа собирается в `[u64; 4]` и пишется
  4 записями вместо 8 (символ — 64 store вместо 128), `draw_extended_char`
  конвертирует цвета 2 раза на символ вместо 128, курсор пишется пакетно.
  `draw_char_at` пропускает перерисовку при совпадении `TextCell`.
  Подробности и стоимость — в [framebuffer.md](framebuffer.md).
- [x] COM1 только в debug: в release `serial::init` не трогает порты,
  `write_serial_byte` пустая, IRQ4 замаскирован, `isr_serial` не ставится.
  Экранный boot-лог не зависит от UART. Цели: `make build-release`,
  `make iso-release`; `./build.sh release` → `make run RELEASE=1 SERIAL_DEBUG=0`.
- [x] Makefile: `RELEASE=1` действительно собирает release-профиль
  (`CARGO_PROFILE=--release` в `kernel:`/`uefi-usb:`/`boot-log-kernel:`),
  `build-release`/`run-release` стали обёртками над `build`, а
  `install-kernel` берёт `$(KERNEL_ELF)` вместо жёсткого `target/release`.
- [x] Прокрутка консоли: `scroll()` выбирает стратегию по типу памяти за
  фреймбуфером (`probe_framebuffer_is_ram`, `rdtsc`-замер чтения). Для RAM —
  `memmove`, для WC/uncached MMIO — перерисовка из текстовой тени со склейкой
  пробельных прогонов. Убирает деградацию «строка за строкой» на реальном
  железе. Подробности — в [framebuffer.md](framebuffer.md).
- [x] `quit` через ACPI S5: разбор FADT (`PM1a_CNT`, `PM1b_CNT`, DSDT) и
  пакета `_S5` в DSDT (включая `Zero`/`One`/`BytePrefix`/`WordPrefix`),
  затем fallback на порты эмуляторов и ACPI reset. `reboot` переведён на
  ACPI reset. Диагностика — `diag power`. Подробности —
  в [storage_and_power.md](storage_and_power.md).
- [x] Каталоги FAT32: `mkdir` (кластер, `.`/`..`, 8.3-запись) и `rmdir`
  (только пустой, корень защищён); `vfs_mkdir`/`vfs_rmdir`/`vfs_remove`,
  `create_dir`/`remove_entry`/`remove_dir` идут через VFS, а не только в
  RamFS; команда `rmdir`.
- [x] Единый корневой RamFS: VFS `/` и legacy-API видят один экземпляр ФС;
  `RamFsFile` пишет в каталог, а не в копию `Vec`. Регрессия —
  `make ramfs-test` (`ls /`, `touch`, `cp`, `hostname`, `/var/log/boot.log`).
- [x] Read-only по умолчанию для FAT32 и `auto`; запись — только явным `rw`.
  Флаг `MountPoint.readonly` проверяется в VFS (`vfs_open/create/mkdir/remove`).
  Регрессия — `make ro-mount-test` (образ не меняется, `COPY.TXT` не появляется).
- [x] Устойчивость к повреждённым ФС: checked-чтение полей NTFS/ext2/GPT,
  лимит на чтение файла целиком; регрессия — `make bad-fs-test`.
- [x] Boot-лог больше не печатается в serial повторно (128 КиБ на реальном
  UART); полный heap-стресс вынесен в `diag heap-test`, на загрузке — короткий.
- [x] NVMe подключён к block layer и boot-log: Identify Controller/Namespace,
  I/O queues, 512/4096-байтные сектора, `mount auto` и VIBRALOG на NVMe.
  Регрессия — `make nvme-test` (QEMU NVMe, включая 4Kn).
- [x] `reboot`: FADT RESET_REG → 0xCF9 → 0x92 → i8042 → triple fault.
- [x] Boot-лог хранится в одном месте: `/var/log/boot.log` (без дублей
  `/boot.log` и `/var/log/kernel.log`).
- [x] Shell: Tab для команд и путей, история без затирания prompt, цвет prompt
  сохраняется, курсор корректно перерисовывается; `Ctrl+C`/`Ctrl+Z`.
- [x] `beep` использует монотонные часы и корректный делитель PIT; `test`
  объединяет системные проверки; `cmd --help` работает для всех команд.
- [x] Запись логов: дисковый приёмник `VIBRALOG` собирается всегда (был под
  feature `boot-log-to-usb`); исправлен троттлинг `attach()` после boot;
  команды `log sink`/`log flush`/`log path`; `quit`/`reboot` сохраняют лог.
- [ ] Framebuffer back buffer (legacy GUI, не блокер kernel-only сборки):
  исправить `flip()` без `pitch`, рассогласование смещений back buffer при
  виртуальном разрешении, `clear()` без back buffer, `draw_cursor` с
  `!bg_color`, отсутствие `color_to_native` в `read_pixel`/`blit`.
  Список — в [framebuffer.md](framebuffer.md).
- [x] QEMU UEFI smoke-test в CI; workflow выполняет `./build.sh smoke`.
- [x] Загрузка ISO (`make iso-smoke`) в UEFI и BIOS в CI; цель выполняется
  в workflow.
- [ ] 100 последовательных QEMU boots: matrix runner и CI job добавлены; локально
  подтверждены 2 последовательных boot на `build/hdd.img` и 1 boot GPT/ESP-образа
  (`BOOT_TIMEOUT=90`). В CI по умолчанию 10 итераций (TCG без KVM), 100 задаются
  через `workflow_dispatch` → `boot_iterations`.
- [x] GPT/FAT32 ESP UEFI test image target (`./build.sh uefi-usb`); `sfdisk`
  verification и OVMF boot прошли.
- [x] Проверяемые PMM/heap stress-тесты: boot heap stress и `diag pmm` прошли
  в QEMU; тест проверяет frame invariants, contiguous alloc/free и DMA buffer.
- [x] Подключить DMA buffer allocator к e1000 RX/TX rings и buffers; QEMU
  подтвердил инициализацию и ARP TX/RX.
- [ ] Интегрировать DMA allocator в блочные драйверы и определить ownership и
  cache-coherency contract; ICMP gateway reply пока не подтверждён.
- [ ] Hardware matrix: 10 boot с отдельного тестового USB на реальной
  x86_64-машине; запуск пока не выполнен.

**Выход:** 100 последовательных QEMU boot и 10 boot на тестовом ПК без
повреждения носителя.

## 0.3.0 «Hardware Core» — прерывания и блочный слой

- [ ] Завершить PIC → LAPIC/IOAPIC для BSP; определить политику MSI/MSI-X.
- [x] Часы и время на реальном железе: TSC (invariant, калибровка по HPET
  или PIT ch2) → HPET (частота из ID-регистра, 64-битный счётчик) → PIT IRQ;
  `uptime`/`date`/тайм-ауты больше не зависят от частоты IRQ-тиков.
- [x] Синхронизация с BIOS: CMOS RTC (UIP, BCD, 12/24h, CENTURY из FADT)
  фиксируется на старте как база wall clock; `date`, `date hw`, `diag timer`.
- [x] `timer::post_init()` измеряет частоту IRQ-тиков; если IRQ0 не
  доставляется, поднимает LAPIC timer (vector 48 + EOI), иначе
  `timer::idle_wait()` не уходит в `hlt` — shell не зависает на ноутбуке.
- [x] Общий `BlockDevice`: bounds, timeouts, read-only, ошибки и DMA-буферы.
  `PartitionDisk` ограничивает окно раздела, AHCI-команды имеют spin-timeout,
  ошибки проверяются по `PxTFD.ERR`, запись разрешена только через явный API.
- [x] GPT/MBR parser и partition devices (`fs/partition.rs`, CRC32 для GPT).
- [x] Один контроллер хранения: AHCI **или** NVMe, только чтение
  (AHCI: Command Header/FIS приведены к AHCI 1.3, IDENTIFY/READ работают).
- [x] Привязать найденные диски к VFS: `fs/blockdev.rs` регистрирует `sda`,
  `sda1`, ... в `DiskManager`, `mount <path> auto <dev>` определяет ФС.

## 0.4.0 «Peripherals» — ввод и файловая система

- [x] Полный xHCI lifecycle: BIOS handoff, reset, DCBAA + scratchpad,
  command/event rings (ERST), port reset, Enable Slot, Address Device,
  GET_DESCRIPTOR, SET_CONFIGURATION, Configure Endpoint и interrupt IN.
  Проверено в QEMU (`make keyboard-test`: `[xHCI] HID keyboard ready`,
  HID-отчёты доходят до драйвера).
- [x] USB HID boot keyboard: 8-байтовые отчёты разбираются в
  `drivers/usb/hid.rs` и инжектятся в общий keyboard-слой. Физическая
  клавиатура ещё не проверена — нужен boot с USB-носителя на ноутбуке.
- [x] PS/2-клавиатура на реальном железе: ACK/self-test 8042, включение
  scanning (0xF4) и порта (0xAE), декодирование Scan Code Set 2 при
  отсутствии трансляции, polling fallback через порт 0x64 (работает без
  IRQ1), отсутствие `println!` из ISR (дедлок на spin-локах). Проверено
  в QEMU: `make keyboard-test` → `KEYBOARD_CASE=ps2`.
- [x] FAT32 read-only, затем Ext2 read-only поверх общего block layer
  (`mount /mnt fat32 sda1`, `mount /mnt ext sda2`); NTFS read-only добавлен
  как бонус (`fs/ntfs.rs`: MFT, data runs, $INDEX_ROOT/$INDEX_ALLOCATION).
  Сквозная проверка — `make fs-test` (`scripts/qemu-fs-test.sh`): FAT32/ext2/NTFS
  и MBR-раздел монтируются, `ls`/`cat` читают файлы, включая LFN.
- [x] Автомонтирование явно выбранного read-only boot-раздела
  (`mount /mnt auto sda1`, FAT32/ext2/NTFS определяются по сигнатурам).

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
