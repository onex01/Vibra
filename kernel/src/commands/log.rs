// log — вывод логов на экран и управление приёмниками.
//
// Логи живут в двух кольцевых буферах (`boot_log`):
//   * boot-лог — всё, что печаталось с момента старта ядра;
//   * журнал shell — всё, что попало в консоль после входа в оболочку
//     (prompt, команды, их вывод). Это «история терминала».
//
// На машине без COM-порта это единственный способ увидеть, что произошло.
// На носитель оба лога пишутся инкрементально: VIBRA.LOG и SHELL.LOG на
// разделе VIBRALOG либо на загрузочном томе.
//
// Подкоманды:
//   log            — последние 40 строк boot-лога
//   log <N>        — последние N строк boot-лога
//   log flush      — принудительный checkpoint (ramfs + носитель)
//   log sink       — куда сейчас пишется лог
//   log path       — путь файлов-приёмников
//   log scan       — какие тома видны и почему не подошли
//   log attach     — повторить поиск приёмника
//   log session    — последние строки журнала shell
//   log session <N>

use crate::framebuffer::{Console, COLOR_CYAN, COLOR_GREEN, COLOR_RED, COLOR_YELLOW};
use super::CmdResult;
use alloc::format;
use alloc::string::String;

const DEFAULT_LINES: usize = 40;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    match args.first().copied() {
        Some("flush") => return flush(console),
        Some("sink") => return sink(console),
        Some("path") => return path(console),
        Some("scan") => return scan(console),
        Some("attach") => return attach(console),
        Some("session") => {
            let lines = args
                .get(1)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(DEFAULT_LINES)
                .max(1);
            return print_session(console, lines);
        }
        _ => {}
    }

    let lines = args
        .first()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LINES)
        .max(1);

    print_boot(console, lines);
    CmdResult::Ok
}

fn print_boot(console: &mut Console, lines: usize) {
    let raw = crate::boot_log::get_log_bytes();
    if raw.is_empty() {
        console.print_colored("boot log is empty\n", COLOR_YELLOW);
        return;
    }

    let text = String::from_utf8_lossy(&raw);
    let all: alloc::vec::Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);

    console.print_colored("[BOOT LOG]\n", COLOR_YELLOW);
    console.print(&format!(
        "  {} bytes, {} lines total; {} of {} lines shown\n",
        raw.len(),
        all.len(),
        all.len() - start,
        all.len()
    ));

    for line in &all[start..] {
        if crate::poll_cancel() {
            console.print_colored("log: cancelled\n", COLOR_YELLOW);
            break;
        }
        console.print(line);
        console.print("\n");
    }
}

fn print_session(console: &mut Console, lines: usize) -> CmdResult {
    let raw = crate::boot_log::session_bytes();
    if raw.is_empty() {
        console.print_colored(
            "shell log is empty (capture starts when the shell is entered)\n",
            COLOR_YELLOW,
        );
        return CmdResult::Ok;
    }

    let text = String::from_utf8_lossy(&raw);
    let all: alloc::vec::Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);

    console.print_colored("[SHELL LOG]\n", COLOR_YELLOW);
    console.print(&format!(
        "  {} bytes, {} lines total\n",
        raw.len(),
        all.len()
    ));
    for line in &all[start..] {
        if crate::poll_cancel() {
            console.print_colored("log: cancelled\n", COLOR_YELLOW);
            break;
        }
        console.print(line);
        console.print("\n");
    }
    CmdResult::Ok
}

/// Принудительный сброс лога на все приёмники.
fn flush(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] flushing...\n", COLOR_CYAN);

    let boot_before = crate::boot_log::usb::written();
    let shell_before = crate::boot_log::usb::shell_written();

    crate::boot_log::flush_all_files();
    crate::boot_log::checkpoint();

    let boot_after = crate::boot_log::usb::written();
    let shell_after = crate::boot_log::usb::shell_written();

    console.print_colored("  ramfs: ", COLOR_GREEN);
    console.print("/var/log/boot.log, /var/log/shell.log\n");

    if crate::boot_log::usb::is_attached() {
        console.print_colored("  volume: ", COLOR_GREEN);
        console.print(&format!(
            "{} bytes boot ({} new), {} bytes shell ({} new)\n",
            boot_after,
            boot_after.saturating_sub(boot_before),
            shell_after,
            shell_after.saturating_sub(shell_before)
        ));
    } else {
        console.print_colored("  volume: ", COLOR_YELLOW);
        console.print("not attached (see `log scan`)\n");
    }

    CmdResult::Ok
}

/// Куда сейчас пишется лог.
fn sink(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] sinks\n", COLOR_CYAN);
    console.print(&format!(
        "  ramfs    : /var/log/boot.log ({} bytes), /var/log/shell.log ({} bytes)\n",
        crate::boot_log::total_len(),
        crate::boot_log::session_total()
    ));

    for line in crate::boot_log::usb::describe() {
        console.print("  ");
        console.print(&line);
        console.print("\n");
    }

    console.print_colored("  debugcon : ", COLOR_GREEN);
    console.print("port 0xE9 ([TICK]/[HALT]/[HEARTBEAT])\n");

    CmdResult::Ok
}

/// Путь файлов-приёмников на носителе.
fn path(console: &mut Console) -> CmdResult {
    if crate::boot_log::usb::is_attached() {
        console.print_colored("boot : ", COLOR_GREEN);
        console.print(crate::boot_log::usb::LOG_FILE);
        console.print("\nshell: ");
        console.print(crate::boot_log::usb::SHELL_FILE);
        console.print("\n");
    } else {
        console.print_colored(
            "no log volume attached; logs are only in ramfs (/var/log/)\n",
            COLOR_RED,
        );
    }
    CmdResult::Ok
}

/// Какие тома видны и почему не подошли.
fn scan(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] scanning volumes\n", COLOR_CYAN);
    for line in crate::boot_log::usb::scan() {
        console.print(&line);
        console.print("\n");
    }
    CmdResult::Ok
}

/// Повторить поиск приёмника (флешку можно воткнуть после загрузки).
fn attach(console: &mut Console) -> CmdResult {
    console.print_colored("[LOG] searching for a log volume...\n", COLOR_CYAN);
    // Сбрасываем троттлинг, чтобы попытка была немедленной.
    crate::boot_log::usb::force_attach();
    if crate::boot_log::usb::is_attached() {
        console.print_colored("  attached\n", COLOR_GREEN);
    } else {
        console.print_colored("  not found\n", COLOR_YELLOW);
    }
    CmdResult::Ok
}
