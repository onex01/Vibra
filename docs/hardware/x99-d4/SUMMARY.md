# Паспорт железа: Intel Atermiter X99 D4

Снято скриптом `scripts/hw-probe.sh` v1.1, 2026-11-06T07:31:16Z (UTC).

Данные собраны **на Linux**, а не в Vibra Kernel: это описание того,
что физически стоит в машине. Результаты прогонов Vibra живут
отдельно, в `docs/verified-hardware.md`.

## Машина

| Поле | Значение |
|---|---|
| Производитель | Intel |
| Модель | Atermiter X99 D4 |
| Материнская плата | Atermiter X99 D4 |
| BIOS/UEFI | American Megatrends Inc. 5.11 |
| Прошивка от | 10/27/2023 |
| Режим загрузки | UEFI |
| Secure Boot | not supported by firmware |
| CPU | Intel(R) Xeon(R) CPU E5-2680 v3 @ 2.50GHz |
| Ядер/потоков | 24 |
| RAM | 32681760 kB |
| Ядро Linux | 7.2.14-200.fc44.x86_64 |
| Дистрибутив | Fedora Linux 44 (Workstation Edition) |

## USB-контроллеры

| PCI | Тип | ID | Драйвер Linux | BAR0 |
|---|---|---|---|---|
| 0000:00:14.0 | xHCI (USB 3.x) | `8086:8c31` | xhci_hcd | `0x0000383ffff00000` |
| 0000:00:1a.0 | EHCI (USB 2.0) | `8086:8c2d` | ehci-pci | `0x00000000fbf02000` |
| 0000:00:1d.0 | EHCI (USB 2.0) | `8086:8c26` | ehci-pci | `0x00000000fbf01000` |

### Регистры (чтение /dev/mem)

```text
[0000:00:14.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
[0000:00:1a.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
[0000:00:1d.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
MMIO не читается. Причины и что делать:
  CONFIG_IO_STRICT_DEVMEM=y — /dev/mem закрыт для ресурсов,
  занятых драйвером. Загрузитесь с параметром ядра iomem=relaxed
  (GRUB: e -> дописать в строку linux -> F10).
```

## USB-устройства

| Путь | VID:PID | Мбит/с | USB | Название |
|---|---|---|---|---|
| 1-1 | 8087:8008 | 480 | 2.00 | — |
| 2-1 | 8087:8000 | 480 | 2.00 | — |
| 3-4 | 25a7:fa61 | 12 | 1.10 | Compx 2.4G Receiver |
| 3-5 | 342d:e40f | 12 | 2.00 | GSKY RSQ-20063 |
| 3-6 | 0c76:9688 | 12 | 1.10 | ME6S c1 ME6S |
| 3-8 | 0a12:0001 | 12 | 1.10 | USB2.0-BT |

### Дерево (`lsusb -t`)

```text
/:  Bus 001.Port 001: Dev 001, Class=root_hub, Driver=ehci-pci/2p, 480M
    |__ Port 001: Dev 002, If 0, Class=Hub, Driver=hub/6p, 480M
/:  Bus 002.Port 001: Dev 001, Class=root_hub, Driver=ehci-pci/2p, 480M
    |__ Port 001: Dev 002, If 0, Class=Hub, Driver=hub/8p, 480M
/:  Bus 003.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/15p, 480M
    |__ Port 004: Dev 002, If 0, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 004: Dev 002, If 1, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 005: Dev 003, If 0, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 005: Dev 003, If 1, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 005: Dev 003, If 2, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 006: Dev 004, If 0, Class=Audio, Driver=snd-usb-audio, 12M
    |__ Port 006: Dev 004, If 1, Class=Audio, Driver=snd-usb-audio, 12M
    |__ Port 006: Dev 004, If 2, Class=Audio, Driver=snd-usb-audio, 12M
    |__ Port 006: Dev 004, If 3, Class=Human Interface Device, Driver=usbhid, 12M
    |__ Port 008: Dev 005, If 0, Class=Wireless, Driver=btusb, 12M
    |__ Port 008: Dev 005, If 1, Class=Wireless, Driver=btusb, 12M
/:  Bus 004.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/6p, 5000M
```

## Контроллеры хранения

| PCI | Тип | ID | Драйвер Linux |
|---|---|---|---|
| 0000:00:1f.2 | SATA (AHCI) | `8086:8c02` | ahci |
| 0000:02:00.0 | NVMe | `1cc1:633a` | nvme |

Линки SATA из dmesg:

```text
[    0.881244] ata6: SATA link up 6.0 Gbps (SStatus 133 SControl 300)
[    0.932726] ata2: SATA link up 6.0 Gbps (SStatus 133 SControl 300)
```

## Накопители

| Устройство | Размер | Шина | Лог. сектор | Физ. сектор | Модель |
|---|---|---|---|---|---|
| sda | 465,8G | sata | 512 | 4096 | HGST HTS545050A7E680 |
| sdb | 1,8T | sata | 512 | 4096 | WDC WD20EZRX-00D8PB0 |
| nvme0n1 | 476,9G | nvme | 512 | 512 | ADATA LEGEND 900 |

## Что не удалось снять

Всё снято.

## Файлы

```
SUMMARY.md      этот файл
raw/            сырые логи (серийники, UUID и MAC отредактированы)
```
