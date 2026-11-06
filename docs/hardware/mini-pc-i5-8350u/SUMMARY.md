# Паспорт железа: unknown-machine

Снято скриптом `scripts/hw-probe.sh` v1.1, 2026-11-06T07:26:18Z (UTC).

Данные собраны **на Linux**, а не в Vibra Kernel: это описание того,
что физически стоит в машине. Результаты прогонов Vibra живут
отдельно, в `docs/verified-hardware.md`.

## Машина

| Поле | Значение |
|---|---|
| Производитель | Default string |
| Модель | Default string |
| Материнская плата | Default string |
| BIOS/UEFI | American Megatrends Inc. N67U106MG |
| Прошивка от | 06/15/2026 |
| Режим загрузки | UEFI |
| Secure Boot | disabled |
| CPU | Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz |
| Ядер/потоков | 8 |
| RAM | 8041800 kB |
| Ядро Linux | 6.12.113+deb13-amd64 |
| Дистрибутив | Debian GNU/Linux 13 (trixie) |

## USB-контроллеры

| PCI | Тип | ID | Драйвер Linux | BAR0 |
|---|---|---|---|---|
| 0000:00:14.0 | xHCI (USB 3.x) | `8086:9d2f` | xhci_hcd | `0x00000000df210000` |

### Регистры (чтение /dev/mem)

```text
[0000:00:14.0] MMIO НЕДОСТУПЕН: PermissionError: [Errno 1] Operation not permitted
MMIO не читается. Причины и что делать:
  CONFIG_IO_STRICT_DEVMEM=y — /dev/mem закрыт для ресурсов,
  занятых драйвером. Загрузитесь с параметром ядра iomem=relaxed
  (GRUB: e -> дописать в строку linux -> F10).
```

## USB-устройства

| Путь | VID:PID | Мбит/с | USB | Название |
|---|---|---|---|---|
| 1-2 | 7825:a2a4 | 480 | 2.10 | ULT-Best Best USB Device |
| 1-7 | 8087:0a2b | 12 | 2.00 | — |
| 2-6 | 0bda:9201 | 5000 | 3.20 | Realtek RTL9201 |

### Дерево (`lsusb -t`)

```text
/:  Bus 001.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/12p, 480M
    |__ Port 002: Dev 002, If 0, Class=Mass Storage, Driver=uas, 480M
    |__ Port 007: Dev 004, If 0, Class=Wireless, Driver=btusb, 12M
    |__ Port 007: Dev 004, If 1, Class=Wireless, Driver=btusb, 12M
/:  Bus 002.Port 001: Dev 001, Class=root_hub, Driver=xhci_hcd/6p, 5000M
    |__ Port 006: Dev 002, If 0, Class=Mass Storage, Driver=uas, 5000M
```

## Контроллеры хранения

| PCI | Тип | ID | Драйвер Linux |
|---|---|---|---|
| 0000:00:17.0 | SATA (AHCI) | `8086:9d03` | ahci |

## Накопители

| Устройство | Размер | Шина | Лог. сектор | Физ. сектор | Модель |
|---|---|---|---|---|---|
| sda | 238,5G | sata | 512 | 512 | SOYO SATA M.2 SY500 256GB |
| sdb | 465,8G | usb | 512 | 4096 | WDC WD5000AAKX-00ERMA0 |
| sdc | 465,8G | usb | 512 | 4096 | HGST HTS725050B7E630 |

## Что не удалось снять

Всё снято.

## Файлы

```
SUMMARY.md      этот файл
raw/            сырые логи (серийники, UUID и MAC отредактированы)
```
