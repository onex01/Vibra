# vktst — Vibra Kernel Tests

`vktst` — единая точка входа для системных проверок. Заменяет повседневное
использование `diag`/`test`/`systest`, оставляя их как алиасы для старых
сценариев и CI.

## Режимы

```text
vktst                 интерактивное меню
vktst list            список тестов
vktst run <id|all>    неинтерактивный прогон
vktst report          прогнать всё и сохранить отчёт
vktst help            краткая справка
```

Интерактивное меню принимает цифру теста, `a` (все), `l` (все + отчёт),
`q` (выход). `Ctrl+C`/`Ctrl+Z` также выходят.

## Группы

| id | Что проверяет |
|---|---|
| `cpu` | топология (physical/logical/threads), vendor, family/model, частоты, флаги |
| `mem` | PMM/heap инварианты и короткий heap-стресс (128 итераций) |
| `timer` | монотонные часы идут, источник (TSC/HPET/PIT), IRQ-тики, RTC |
| `fs` | корневой RamFS, `/proc`, запись/чтение `/tmp/vktst-probe.txt`, список монтирований |
| `disk` | блочные устройства, разделы, `fs_type`, счётчики I/O |
| `net` | MAC/IP и счётчики RX/TX |
| `kernel` | версия, число devices/drivers/modules |
| `log` | `/var/log/boot.log` и состояние USB-приёмника |

## Отчёт

`vktst report` и пункт `l` пишут файл:

```text
/var/log/vktst-<uptime-секунды>.log
```

Формат — тот же, что печатается на экран: `[ OK ]`/`[FAIL]`, детали и
итоговая строка `summary: N passed, M failed`. Файл создаётся через VFS,
поэтому его можно прочитать командой `cat` или снять с образа хостовыми
утилитами.

## CI

`make vktst-test` (`scripts/qemu-vktst-test.sh`) запускает
`vktst run all`, `free`, `top -b` и `tree` из `/proc`, затем проверяет
маркеры в serial-логе. Тест входит в `.github/workflows/kernel.yml`.

## Отношение к старым командам

- `diag` остаётся для глубокой диагностики подсистем (`diag timer`,
  `diag pmm`, `diag heap-test`, `diag fault-test`, ...).
- `test` и `systest` не удалены, чтобы не ломать существующие скрипты, но
  для интерактивной проверки используйте `vktst`.
