# USB и загрузка

## Реальный статус USB

В `0.1.0` USB не является рабочим вводом или накопителем. Код xHCI умеет
находить PCI-контроллер и выводить диагностические регистры. Очереди и тестовые
TRB не равны перечислению устройств; `hid_ok` — искусственная инъекция для
проверки очереди клавиатуры, а не HID-драйвер.

До теста на реальном оборудовании не включайте USB в обязательный boot path.
План: корректный reset и BIOS handoff → DCBAA/command/event rings → port reset
→ Address Device → descriptors/configuration → HID boot keyboard.

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

Сначала проверьте `build/vibra-uefi.img` в QEMU/OVMF. Скрипт никогда не пишет
образ на физическое устройство. Для реального запуска вручную выберите
отдельный тестовый USB-носитель, сверьте его модель/размер и запишите образ
только после проверки имени устройства. Не используйте носитель с данными.

## Host dependencies

`./build.sh doctor` определяет Debian/Ubuntu и Fedora/RHEL-подобные системы и
показывает команду установки пакетов. Инструкции вручную приведены в корневом
README.
