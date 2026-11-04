#!/usr/bin/env bash
# test-build-sh.sh — проверка хостового интерфейса ./build.sh.
#
# Что ловит:
#   1. синтаксис и запуск меню (в том числе неинтерактивно, из pipe);
#   2. каждая команда из таблицы действительно ведёт на существующую
#      make-цель (опечатка в имени цели иначе всплывает только в CI);
#   3. каждая команда, которую вызывают workflow из .github/workflows,
#      есть в таблице — это главный источник «переименовали и забыли»;
#   4. неизвестная команда даёт код 2 и подсказку, а не тихо что-то делает.
#
# Запуск: bash scripts/test-build-sh.sh  (нужен только bash + make)
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail() {
    echo "build.sh test failed: $1" >&2
    exit 1
}

echo "==> Syntax"
bash -n build.sh || fail "build.sh has a syntax error"

echo "==> Command table maps to real make targets"
# Имена берём из вывода `list`: это тот же путь, что видят пользователь и
# CI, поэтому проверка не может разойтись с таблицей из-за разбора текста.
commands="$(./build.sh list | sed -n 's/^  \([a-z0-9-]*\) *$/\1/p')"

[[ -n "$commands" ]] || fail "cannot read the command table from build.sh"

checked=0
skipped=0
for cmd in $commands; do
    case "$cmd" in
        doctor|read-boot-log)
            # Без make-цели: только проверки хоста / передача устройства.
            skipped=$((skipped + 1))
            continue
            ;;
    esac
    out="$(DRY_RUN=1 ./build.sh "$cmd" 2>&1)" || fail "'./build.sh $cmd' exited non-zero"
    make_line="$(printf '%s\n' "$out" | grep -m1 '^make ' || true)"
    if [[ -z "$make_line" ]]; then
        fail "'./build.sh $cmd' did not resolve to a make command (got: ${out:-<empty>})"
    fi
    # make_line уже содержит ведущий "make": подставляем -n после него.
    # shellcheck disable=SC2086
    if ! make -n ${make_line#make } >/dev/null 2>&1; then
        fail "'./build.sh $cmd' maps to a missing make target: $make_line"
    fi
    checked=$((checked + 1))
done
echo "  checked $checked command(s), $skipped without a make target"

echo "==> Every command used by CI exists"
ci_commands="$(
    grep -rhoE '\./build\.sh [a-z0-9-]+' .github/workflows/*.yml 2>/dev/null \
        | awk '{print $2}' | sort -u
)"
[[ -n "$ci_commands" ]] || fail "no './build.sh <cmd>' calls found in the workflows"
for cmd in $ci_commands; do
    if ! printf '%s\n' "$commands" | grep -qx "$cmd"; then
        fail "workflow calls './build.sh $cmd' but it is not in the command table"
    fi
done
echo "  $(printf '%s\n' "$ci_commands" | wc -l | tr -d ' ') CI command(s) verified"

echo "==> Every menu entry calls a real command"
# Пункты меню жёстко пронумерованы и ссылаются на команды из таблицы: если
# команду переименовать, пункт молча начнёт падать уже у пользователя.
# Только строки вида "  12) run_command foo ;;" — упоминания в комментариях
# не считаются пунктами меню.
menu_commands="$(grep -oE '^ *[0-9]+\) run_command [a-z0-9-]+' build.sh | awk '{print $3}' | sort -u)"
[[ -n "$menu_commands" ]] || fail "no run_command calls found in build.sh"
for cmd in $menu_commands; do
    if ! printf '%s\n' "$commands" | grep -qx "$cmd"; then
        fail "menu calls run_command $cmd but it is not in the command table"
    fi
done
echo "  $(printf '%s\n' "$menu_commands" | wc -l | tr -d ' ') menu command(s) verified"

echo "==> list / help / unknown"
# Вывод сначала в переменную: `grep -q` закрывает pipe, и с `pipefail`
# скрипт падал бы на SIGPIPE самого ./build.sh.
list_out="$(./build.sh list)"
printf '%s\n' "$list_out" | grep -q '^  build ' || fail "'list' does not show 'build'"
help_out="$(./build.sh help)"
printf '%s\n' "$help_out" | grep -q 'Usage: ./build.sh' || fail "'help' prints no usage"
status=0
./build.sh definitely-not-a-command >/dev/null 2>&1 || status=$?
[[ "$status" -eq 2 ]] || fail "unknown command must exit 2, got $status"

echo "==> Menu renders and quits"
menu_out="$(printf '6\n' | ./build.sh menu 2>&1)"
printf '%s' "$menu_out" | grep -q 'Build images' || fail "main menu does not render"
printf '%s' "$menu_out" | grep -q 'Quit' || fail "main menu has no Quit entry"
# Все подменю должны открываться и закрываться, не зацикливаясь.
for sub in '1:5' '2:5' '3:10' '4:21' '5:7'; do
    entry="${sub%%:*}"
    back="${sub##*:}"
    out="$(printf '%s\n%s\n6\n' "$entry" "$back" | ./build.sh menu 2>&1)" \
        || fail "submenu $entry/$back exited non-zero"
    printf '%s' "$out" | grep -q 'What do you want to do' \
        || fail "submenu $entry did not return to the main menu"
done

echo "build.sh test passed"
