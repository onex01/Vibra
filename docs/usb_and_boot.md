# USB и загрузка

> Инженерные заметки о текущем состоянии USB — в
> [usb_progress.md](usb_progress.md). Этот файл описывает образы и процедуры
> проверки; при расхождении приоритет у `usb_progress.md` и кода.

## Реальный статус USB

xHCI, UHCI и EHCI проходят полный lifecycle в QEMU; xHCI/UHCI дают ещё и
ввод (HID boot keyboard), а все три — накопители через BOT + SCSI.
`make keyboard-test`, `make uhci-test`, `make usb3-test`,
`make usb-storage-test` и `make ehci-test` проверяют это автоматически.

EHCI важен для ноутбуков Haswell/Broadwell: USB2-половина разъёмов висит на
EHCI, и без драйвера флешка не видна ни в одном порту (на T450 — все три).
Драйвер появился в `rc.25`, но на реальном железе ещё не подтверждён (см.
[verified-hardware.md](verified-hardware.md)).

До теста на реальном оборудовании не включайте USB в обязательный boot path.

## QEMU

```sh
./build.sh setup
./build.sh debug
./build.sh qemu-usb
make ehci-test          # EHCI + usb-storage, монтирование и hotplug
```

`qemu-usb` добавляет эмулируемые xHCI keyboard/mouse только для разработки.
`make ehci-test` поднимает `usb-ehci` и USB2-накопитель. Смотрите serial log;
успешная загрузка QEMU не подтверждает физический USB.

## Образы

`./build.sh iso` и `./build.sh usb` создают экспериментальные образы. Для
UEFI-only теста x86_64 используйте `./build.sh uefi-usb`: он создаёт GPT disk
с FAT32 EFI System Partition и fallback loader
`EFI/BOOT/BOOTX64.EFI`. На тестовой машине отключите Secure Boot.

Имена образов версионированы: `vibra_<DATE>_<VERSION>.iso`,
`vibra-debug_<DATE>_<VERSION>.iso`, `vibra-uefi_<DATE>_<VERSION>.img`,
`vibra-usb_<DATE>_<VERSION>.img`,
`vibra-usb-debug_<DATE>_<VERSION>.img`. Симлинки без даты
(`build/vibra.iso` и т.д.) сохранены для скриптов.

### Debug-образ с тремя разделами

`make usb-debug` создаёт GPT-образ:

1. `VIBRA-ESP` — FAT32 с Limine;
2. `VIBRA` — FAT32 с `kernel.elf`; Limine грузит его по метке
   (`fslabel(VIBRA):/kernel.elf`);
3. `VIBRALOG` — FAT32 с предвыделенным `/VIBRA.LOG`.

`make iso-debug` добавляет те же `VIBRA` и `VIBRALOG` к гибридному ISO через
`xorriso -append_partition`. Такой ISO нужно записать на USB: с CD/DVD лог
физически не записать, и ядро честно сообщает об этом. Проверка —
`make debug-image-test` и `make iso-debug-test`.

`./build.sh iso-smoke` проверяет сам гибридный ISO: загружает его через
`-cdrom` в UEFI (OVMF) и BIOS (SeaBIOS) и требует boot-маркеры и prompt
`vibra:/>` в serial-логе. Это проверка именно съёмочного образа, а не
`build/hdd.img`.

Сначала проверьте `build/vibra-uefi.img` в QEMU/OVMF. Скрипт никогда не пишет
образ на физическое устройство. Для реального запуска вручную выберите
отдельный тестовый USB-носитель, сверьте его модель/размер и запишите образ
только после проверки имени устройства. Не используйте носитель с данными.

## Контроллеры USB

Тип контроллера определяется по PCI `prog_if` (class 0x0C / subclass 0x03):

| prog_if | Тип | Статус |
|---|---|---|
| 0x00 | UHCI (USB 1.1, Intel/VIA) | поддерживается: control, interrupt, bulk |
| 0x10 | OHCI (USB 1.1, NEC/AMD/VIA) | не поддерживается |
| 0x20 | EHCI (USB 2.0) | поддерживается: control, bulk, storage, hotplug |
| 0x30 | xHCI (USB 3.x) | поддерживается: control, bulk, HID, storage, hotplug |

Клавиатуры low-speed на чипсетах Intel и AMD SB подключены к UHCI-компаньону,
поэтому без UHCI-драйвера они не работали вовсе. EHCI нужен для
high-speed-устройств (флешек) на USB2-портах. На большинстве машин с USB3
флешка работает и через xHCI; на Haswell/Broadwell (например, ThinkPad T450)
USB2-порты целиком принадлежат EHCI, и без него устройство не появляется.

## Логи на носителе

Boot-лог и журнал shell пишутся в `VIBRA.LOG` и `SHELL.LOG`:

1. на раздел с меткой `VIBRALOG`, если он есть;
2. иначе на загрузочный том — тот, где лежит `KERNEL.ELF`.

Второй вариант важен для диагностики: лог появляется даже на машине, где
никто не готовил отдельный раздел. `log scan` показывает все найденные тома
и причину отказа, `log attach` повторяет поиск (флешку можно воткнуть позже).

## Host dependencies

`./build.sh doctor` определяет Debian/Ubuntu и Fedora/RHEL-подобные системы и
показывает команду установки пакетов. Инструкции вручную приведены в корневом
README.
