# Паспорт железа: LENOVO Stin20B

Снято скриптом `scripts/hw-probe.sh` v1.1, 2026-11-06T07:53:47Z (UTC).

Данные собраны **на Linux**, а не в Vibra Kernel: это описание того,
что физически стоит в машине. Результаты прогонов Vibra живут
отдельно, в `docs/verified-hardware.md`.

## Машина

| Поле | Значение |
|---|---|
| Производитель | LENOVO |
| Модель | Stin20B |
| Материнская плата | Stin20B |
| BIOS/UEFI | LENOVO JBET73WW (1.37 ) |
| Прошивка от | 08/14/2019 |
| Режим загрузки | UEFI |
| Secure Boot | disabled |
| CPU | Intel(R) Core(TM) i3-5010U CPU @ 2.10GHz |
| Ядер/потоков | 4 |
| RAM | 16239028 kB |
| Ядро Linux | 7.2.11-200.fc44.x86_64 |
| Дистрибутив | Fedora Linux 44 (Workstation Edition) |

## USB-контроллеры

| PCI | Тип | ID | Драйвер Linux | BAR0 |
|---|---|---|---|---|
| 0000:00:14.0 | xHCI (USB 3.x) | `8086:9cb1` | xhci_hcd | `0x00000000e1220000` |
| 0000:00:1d.0 | EHCI (USB 2.0) | `8086:9ca6` | ehci-pci | `0x00000000e123d000` |

### Регистры (чтение /dev/mem)

```text
[0000:00:14.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
[0000:00:1d.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
MMIO не читается. Причины и что делать:
  CONFIG_IO_STRICT_DEVMEM=y — /dev/mem закрыт для ресурсов,
  занятых драйвером. Загрузитесь с параметром ядра iomem=relaxed
  (GRUB: e -> дописать в строку linux -> F10).
```

## USB-устройства

| Путь | VID:PID | Мбит/с | USB | Название |
|---|---|---|---|---|
| 1-1 | 8087:8001 | 480 | 2.00 | — |
| 2-7 | 8087:0a2a | 12 | 2.01 | — |
| 2-8 | 04f2:b449 | 480 | 2.00 | Chicony Electronics Co.,Ltd. Integrated Camera |

### Дерево (`lsusb -t`)

```text
/:  Bus 001.Port 001: Dev 001, Class=root_hub, Driver=ehci-pci/3p, 480M
    |__ Port 001: Dev 002, If 0, Class=Hub, Driver=hub/8p, 480M
/:  Bus 002.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/11p, 480M
    |__ Port 007: Dev 002, If 0, Class=Wireless, Driver=btusb, 12M
    |__ Port 007: Dev 002, If 1, Class=Wireless, Driver=btusb, 12M
    |__ Port 008: Dev 003, If 0, Class=Video, Driver=uvcvideo, 480M
    |__ Port 008: Dev 003, If 1, Class=Video, Driver=uvcvideo, 480M
/:  Bus 003.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/4p, 5000M
```

## Контроллеры хранения

| PCI | Тип | ID | Драйвер Linux |
|---|---|---|---|
| 0000:00:1f.2 | SATA (AHCI) | `8086:9c83` | ahci |

Линки SATA из dmesg:

```text
[    1.189319] ata1: SATA link up 6.0 Gbps (SStatus 133 SControl 300)
```

## Накопители

| Устройство | Размер | Шина | Лог. сектор | Физ. сектор | Модель |
|---|---|---|---|---|---|
| sda | 447,1G | sata | 512 | 512 | XrayDisk 480GB SSD |

## Что не удалось снять

Всё снято.

## Файлы

```
SUMMARY.md      этот файл
raw/            сырые логи (серийники, UUID и MAC отредактированы)
```
