Резюме состояния USB/xHCI в проекте Vibra

Дата: 2026-08-14

Что уже сделано:
- Makefile: опция ENABLE_USB и run-таргеты, serial-over-tcp поддержка.
- Serial: IRQ4 теперь вызывает `serial::handle_interrupt()`; serial ring-buffer работает.
- Keyboard: добавлена HID-очередь и `inject_key()`; `poll_key()` читает HID очередь первой.
- USB обнаружение: `kernel/src/drivers/usb.rs` сканирует PCI, читает CAPLENGTH/HCIVERSION/HCSPARAMS1, выполняет HCRST и читает PORTSC.
- xHCI infra: `kernel/src/drivers/usb/xhci.rs` добавлен, реализует выделение страниц для ASQ/ACQ и запись `AQA/ASQ/ACQ`.
- HID: минимальный модуль `kernel/src/drivers/usb/hid.rs` инъектирует тестовую строку в очередь клавиатуры.
- Команда `hidtest` добавлена для ручной инъекции тестовых клавиш.

Файлы, которые были изменены/созданы (важные):
- kernel/src/drivers/usb.rs
- kernel/src/drivers/usb/xhci.rs
- kernel/src/drivers/usb/hid.rs
- kernel/src/keyboard.rs (inject_key + HID_QUEUE)
- kernel/src/commands/hidtest.rs
- Makefile (USB/serial flags)

Что осталось сделать (рекомендуемая последовательность):
1) xHCI: Event Ring + Command Completion (обработчик событий).
2) xHCI: Реализовать команды Enable Slot, Address Device, настроить Device/Endpoint Contexts.
3) USB HID: парсинг HID-отчётов и мэппинг в `keyboard::Key`.
4) USB MSC: поддержка Mass Storage (опционально, для загрузки с флешки).

Как продолжить (быстрые команды):
- Сборка и запуск QEMU с serial-over-tcp:
  make run-serial-tcp
  telnet localhost 4444
- Просмотр логов: в serial вы увидите print'ы из драйверов ("[USB]", "[xHCI]", "[HID]")

Контекст и советы для следующего ассистента:
- Проверьте `kernel/src/drivers/usb.rs` и `kernel/src/drivers/usb/xhci.rs` для регистрационных смещений.
- Будьте осторожны с прямой записью регистров xHCI: используйте CAPLENGTH и HCSPARAMS1 для вычисления оффсетов.
- Если QEMU не показывает ожидаемые логи, подключитесь по TCP к serial и проверьте наличие `[HID]`/`[xHCI]` сообщений.

Примечание: файл создан автоматически помощником; обновляйте его по мере продвижения.
