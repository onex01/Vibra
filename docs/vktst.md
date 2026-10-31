# vktst — Vibra Kernel Tests

`vktst` — единая точка входа для системных проверок. Команды `test`,
`systest`, `heap`, `hidtest`, `usertest` и `test-disk` удалены в 0.2.0-rc.5:
их функции либо дублировались `vktst`/`diag`/`free`, либо были
разработческими заглушками.

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
| `mem` | PMM/heap инварианты, короткий heap-стресс (128 итераций) и `heap-check` |
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

## Отношение к `diag`

- `vktst` — быстрый обзор и интерактивные проверки.
- `diag` — глубокая диагностика конкретной подсистемы: `diag pmm`,
  `diag paging`, `diag vmm`, `diag wxtest`, `diag timer`, `diag apic`,
  `diag display`, `diag disk`, `diag keyboard`, `diag hw`, `diag usb`,
  `diag power`, `diag heap-test`, `diag fault-test`, `diag panic-test`.
- Дубли (`test`, `systest`, `kstat`, `apic`, `display`, `tasks`) удалены;
  `apic` и `display` переехали в `diag apic` / `diag display`.
