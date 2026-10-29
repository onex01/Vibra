# USB и загрузка

> Инженерные заметки о текущем состоянии xHCI — в
> [usb_progress.md](usb_progress.md). Этот файл описывает образы и процедуры
> проверки; при расхождении приоритет у `usb_progress.md` и кода.

## Реальный статус USB

В `0.1.0` USB не был рабочим вводом или накопителем. Код xHCI умел находить
PCI-контроллер и выводить диагностические регистры. Очереди и тестовые TRB не
равны перечислению устройств; `hid_ok` был искусственной инъекцией для проверки
очереди клавиатуры, а не HID-драйвером.

До теста на реальном оборудовании не включайте USB в обязательный boot path.
План: корректный reset и BIOS handoff → DCBAA/command/event rings → port reset
→ Address Device → descriptors/configuration → HID boot keyboard.

В `0.2.0-rc.2` xHCI enumeration и HID boot keyboard уже работают в QEMU
(`make keyboard-test` проверяет PS/2 и USB-ввод через monitor `sendkey`), но на
физическом железе это по-прежнему не подтверждено.

## QEMU

```sh
./build.sh setup
./build.sh debug
./build.sh qemu-usb
```

`qemu-usb` добавляет эмулируемые xHCI keyboard/mouse только для разработки.
Смотрите serial log; успешная загрузка QEMU не подтверждает физический USB.

## Образы

`./build.sh iso` и `./build.sh usb` создают экспериментальные образы. Для
UEFI-only теста x86_64 используйте `./build.sh uefi-usb`: он создаёт GPT disk
с FAT32 EFI System Partition и fallback loader
`EFI/BOOT/BOOTX64.EFI`. На тестовой машине отключите Secure Boot.

`./build.sh iso-smoke` проверяет сам гибридный ISO: загружает его через
`-cdrom` в UEFI (OVMF) и BIOS (SeaBIOS) и требует boot-маркеры и prompt
`vibra:/>` в serial-логе. Это проверка именно съёмочного образа, а не
`build/hdd.img`.

Сначала проверьте `build/vibra-uefi.img` в QEMU/OVMF. Скрипт никогда не пишет
образ на физическое устройство. Для реального запуска вручную выберите
отдельный тестовый USB-носитель, сверьте его модель/размер и запишите образ
только после проверки имени устройства. Не используйте носитель с данными.

## Host dependencies

`./build.sh doctor` определяет Debian/Ubuntu и Fedora/RHEL-подобные системы и
показывает команду установки пакетов. Инструкции вручную приведены в корневом
README.
