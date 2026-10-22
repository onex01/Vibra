# USB и загрузка

## Реальный статус USB

`0.2.0` больше не ограничивается детекцией: xHCI проходит полный lifecycle
(BIOS handoff → reset → DCBAA/scratchpad → command ring → event ring/ERST →
port reset → Enable Slot → Address Device → дескрипторы → SET_CONFIGURATION →
Configure Endpoint) и умеет работать с USB HID boot keyboard.

Драйвер polling-based: во время boot прерывания выключены, а MSI/MSI-X для
xHCI требует отдельной инфраструктуры. `xhci::poll_keyboard()` вызывается из
`keyboard::poll_key()` и разбирает 8-байтовые HID-отчёты.

Проверка в QEMU:

```sh
make keyboard-test KEYBOARD_CASE=usb
```

Ожидаемые строки в `build/keyboard-usb.log`:

```
[USB] ... xHCI v1.0 (4 ports, CAPLEN=64)
[xHCI] ... legacy handoff: bios_owned=0 os_owned=1
[xHCI] port 1: connected (speed 1 = full)
[xHCI] HID keyboard ready: slot 1 port 1 0627:0001 ep=0x81 mps=8 interval=10
```

Физическая USB-клавиатура ещё не подтверждена: нужен boot с USB-образа на
ноутбуке. До этого не включайте USB в обязательный boot path.

## PS/2 (i8042)

На ноутбуках встроенная клавиатура обычно висит на i8042, а не на USB.
`keyboard.rs` теперь:

- делает self-test 8042, включает порт (0xAE) и scanning (0xF4), ждёт ACK;
- проверяет, принят ли бит translation, и при отказе декодирует Scan Code Set 2;
- опрашивает порт 0x64 в `poll_key()` (polling fallback), поэтому ввод работает,
  даже если IRQ1 не доставляется (ACPI/EC, IOAPIC вместо PIC);
- не печатает из ISR — `println!` из прерывания берёт spin-локи и вешает машину.

Диагностика: `diag keyboard` (состояние контроллера, scan set, счётчик IRQ1,
очередь PS/2 и USB HID, число HID-отчётов).

## QEMU

```sh
./build.sh setup
./build.sh debug
./build.sh qemu-usb
```

`qemu-usb` добавляет эмулируемые xHCI keyboard/mouse только для разработки.
Смотрите serial log; успешная загрузка QEMU не подтверждает физический USB.

## Образы

`./build.sh iso` и `./build.sh usb` создают экспериментальные образы. Скрипты
не прошивают устройства. Перед ручной записью образа на флешку проверьте имя
целевого устройства и используйте отдельный тестовый носитель.

## Host dependencies

`./build.sh doctor` определяет Debian/Ubuntu и Fedora/RHEL-подобные системы и
показывает команду установки пакетов. Инструкции вручную приведены в корневом
README.
