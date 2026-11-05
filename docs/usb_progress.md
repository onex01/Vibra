# USB — реальный статус

> Короткая версия: xHCI, UHCI и EHCI проходят lifecycle в QEMU; USB mass
> storage читается и пишется через VFS. На T450 EHCI — единственный путь
> к флешке; драйвер есть с `rc.25`, но на железе ещё не проверялся (см.
> [verified-hardware.md](verified-hardware.md)).

## Контроллеры

Тип определяется по PCI `prog_if` (class `0x0C`, subclass `0x03`):

| prog_if | Контроллер | Состояние |
|---|---|---|
| `0x00` | UHCI (USB 1.1) | lifecycle, control/interrupt/bulk, HID-клавиатура |
| `0x10` | OHCI (USB 1.1) | нет драйвера |
| `0x20` | EHCI (USB 2.0) | lifecycle, control/bulk, mass storage, hotplug |
| `0x30` | xHCI (USB 3.x) | lifecycle, control/bulk, HID-клавиатура, storage, hotplug |

`diag usb` печатает все найденные контроллеры, их BAR/MMIO, корневые порты
(`PORTSC`: `CCS`, `PED`, `PP`, скорость, `usb2`/`usb3`) и перечисленные
устройства. Для EHCI также выводится `USBLEGSUP`/`CONFIGFLAG` — по ним видно,
отдала ли прошивка порты в руки ОС.

## xHCI

Проходит полный lifecycle: BIOS handoff → `HCRESET` → DCBAA/scratchpad →
command ring → event ring/ERST → port reset → Enable Slot → Address Device →
дескрипторы → `SET_CONFIGURATION` → Configure Endpoint.

- EP0 для SuperSpeed — 512 байт; `bMaxBurst` из Endpoint Companion попадает
  в endpoint context, `max_transfer` считается по burst.
- Hotplug: событие `PORT_STATUS_CHANGE` выставляет флаг, `poll_hotplug`
  перечисляет новые устройства и убирает отключённые; block layer
  пересобирает записи USB-дисков.
- `poll_keyboard()` вызывается из `keyboard::poll_key()` и разбирает
  8-байтовые HID boot-отчёты; прерывания не используются.

Драйвер polling-based: во время boot прерывания выключены, а MSI/MSI-X для
xHCI требует отдельной инфраструктуры (см. `rc.30` в roadmap).

## UHCI

`drivers/usb/uhci.rs` — control, interrupt и bulk через кадровый список
(SOF 1 мс). Нужен на чипсетах Intel/AMD, где low-speed клавиатуры висят на
USB 1.1-компаньоне. `make uhci-test` проверяет USB-клавиатуру на UHCI.

## Mass storage (BOT + SCSI)

`drivers/usb/storage.rs` реализует Bulk-Only Transport и подмножество SCSI:
`INQUIRY`, `TEST UNIT READY`, `READ CAPACITY(10)`, `READ(10)`, `WRITE(10)`,
`REQUEST SENSE`, `GET_MAX_LUN`, BOT reset. Диски попадают в общий block
layer как `sdX`, поэтому `disks`, `mount`, `df` и VFS работают так же, как с
AHCI/NVMe/IDE.

Проверки:

```sh
make usb3-test         # SuperSpeed-диск, монтирование, hotplug
make usb-storage-test  # mount fat32, ls, cat, запись cp через VFS
```

## EHCI (USB 2.0)

На Haswell/Broadwell USB2-половина каждого USB3-разъёма принадлежит EHCI.
На Lenovo T450 оба контроллера есть (`8086:9C31` xHCI, `8086:9CA6` EHCI), но
у xHCI все 15 портов читаются как `PORTSC=0x2a0` (`PP=1`, `CCS=0`) — ни один
не видит подключения. Companion-контроллеров UHCI на этой машине нет.

`drivers/usb/ehci.rs` реализует только async schedule: периодический список
нужен для interrupt-эндпоинтов, а BOT их не использует; split transactions —
только для USB 1.1 за хабом.

- PCI `prog_if 0x20`, BAR0, legacy handoff, `HCRESET`,
  `CONFIGFLAG.CFL = 1`, `USBCMD` Run+ASE.
- QH (48 байт, выравнивание 32) и qTD (32 байта); адреса — через
  `paging::dma_address`, только ниже 4 ГиБ.
- Control transfers и перечисление, bulk IN/OUT, hotplug по `PORTSC.CSC`.
- `storage.rs` один на два контроллера: `UsbHost { Xhci, Ehci }`,
  диспетчеризация `bulk`/`resolve_stall`/`storage_devices`.

Особенности протокола, на которых легко обжечься: Active в qTD token — бит 7;
horizontal link QH требует тип `01`; overlay QH содержит ссылку на первый
qTD, а не сам qTD; запись `PORTSC` без бита `PE` выключает порт.

Проверка: `make ehci-test` (QEMU `usb-ehci` + `usb-storage`, монтирование,
чтение, hotplug).

## PS/2 (i8042)

На ноутбуках встроенная клавиатура обычно висит на i8042, а не на USB.
`keyboard.rs`:

- делает self-test 8042, включает порт (`0xAE`) и scanning (`0xF4`), ждёт ACK;
- проверяет, принят ли бит translation, и при отказе декодирует Scan Code Set 2;
- опрашивает порт `0x64` в `poll_key()` (polling fallback), поэтому ввод
  работает, даже если IRQ1 не доставляется (ACPI/EC, IOAPIC вместо PIC);
- не печатает из ISR — `println!` из прерывания берёт spin-локи и вешает машину.

Диагностика: `diag keyboard` (состояние контроллера, scan set, счётчик IRQ1,
очередь PS/2 и USB HID, число HID-отчётов).

## Проверка в QEMU

```sh
make keyboard-test            # PS/2 и USB HID через monitor sendkey
make keyboard-test KEYBOARD_CASE=usb
make uhci-test                # клавиатура на UHCI
make usb3-test                # SuperSpeed-диск и hotplug
make usb-storage-test         # FAT32 на USB-диске через VFS
make ehci-test                # EHCI-диск: монтирование, чтение, hotplug
```

Ожидаемые строки в `build/keyboard-usb.log`:

```text
[USB] ... xHCI v1.0 (4 ports, CAPLEN=64)
[xHCI] ... legacy handoff: bios_owned=0 os_owned=1
[xHCI] port 1: connected (speed 1 = full)
[xHCI] HID keyboard ready: slot 1 port 1 0627:0001 ep=0x81 mps=8 interval=10
```

QEMU умеет EHCI: `-device usb-ehci,id=ehci -device usb-storage,bus=ehci.0`.
Это стенд `make ehci-test`.

## Что ещё не сделано

- OHCI (USB 1.1, NEC/AMD/VIA) — не начато.
- USB HID на EHCI — не поддержан: там только mass storage.
- Хабы (класс 09) — не нужны, пока устройство подключено к корневому порту.
- MSI/MSI-X и прерывания вместо polling — `rc.30`.
- Несколько USB-накопителей одновременно и стабильные `sdX` между
  переподключениями — `rc.28`.

## Ограничение по железу

Успешная загрузка в QEMU не подтверждает физический USB. Подтверждение —
запись в [verified-hardware.md](verified-hardware.md) после прогона на
реальной машине.